use cfg_if::cfg_if;
cfg_if! {
    if #[cfg(feature="jemalloc")] {
        use jemallocator::Jemalloc;
        #[global_allocator]
        static A: Jemalloc = Jemalloc;
    } else {
        use std::alloc::System;
        #[global_allocator]
        static A: System = System;
    }
}

use clap::{App, Arg, SubCommand};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

mod pandoc;
use pandoc_types::definition::{Inline, MetaValue, Pandoc as PandocDocument};

use citeproc::db::ReferenceDatabase;
use citeproc::db_impl::RootDatabase;
use citeproc::locale::{Lang, LocaleFetcher};
use citeproc::output::*;
use citeproc::Driver;

fn main() {
    // heuristically determine if we're running as an external pandoc filter
    // TODO: work out earliest pandoc that sets PANDOC_VERSION
    let not_a_tty = !atty::is(atty::Stream::Stdin) && !atty::is(atty::Stream::Stdout);
    if std::env::var("PANDOC_VERSION").is_ok() && not_a_tty {
        do_pandoc();
        return;
    }

    let matches = App::new("citeproc")
        .version("0.0.0")
        .author("Cormac Relf")
        .about("Processes citations")
        .subcommand(
            SubCommand::with_name("parse-locale")
                .about("Parses a locale file (without performing fallback)")
                .arg(
                    Arg::with_name("lang")
                        .short("l")
                        .long("lang")
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("disamb-index")
                .about("Prints the inverted disambiguation index for the reference library"),
        )
        // .arg(
        //     Arg::with_name("format")
        //         .short("f")
        //         .long("format")
        //         .value_name("FORMAT")
        //         .takes_value(true),
        // )
        .arg(
            Arg::with_name("library")
                .short("l")
                .long("library")
                .value_name("FILE.json")
                .help("A CSL-JSON file")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("csl")
                .short("c")
                .long("csl")
                .value_name("FILE")
                .help("A CSL style")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("key")
                .short("k")
                .long("key")
                .value_name("CITEKEY")
                .help("Run against a specific citekey")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("locales-dir")
                .long("locales-dir")
                .value_name("DIR")
                .help("Directory with locales-xx-XX.xml files in it")
                .takes_value(true),
        )
        .get_matches();

    let mut lib_text = String::from(
        r#"
    [
        {
            "id": "quagmire2018",
            "type": "legal_case",
            "volume": "2, 4",
            "edition": "128th & 7-9, 17th",
            "page": "1-5",
            "container-title": "TASCC",
            "title": "Solomon v Garrity",
            "author": [
                {"family": "Beethoven", "dropping-particle": "van", "given": "Ludwig"}
            ],
            "editor": [
                {"family": "Paul", "given": "John", "suffix": "II"},
                {"family": "Mozart", "given": "Wolfgang Amadeus"},
                {"family": "Beethoven", "dropping-particle": "van", "given": "Ludwig"}
            ],
            "issued": {"raw": "1995-03-01"}
        }
    ]
    "#,
    );

    let filesystem_fetcher = {
        let locales_dir = matches
            .value_of("locales-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let pd = ProjectDirs::from("net", "cormacrelf", "citeproc-rs")
                    .expect("No home directory found.");
                let mut locales_dir = pd.cache_dir().to_owned();
                locales_dir.push("locales");
                locales_dir
            });
        if matches.subcommand_matches("parse-locale").is_some() {
            let locales_dir = locales_dir.clone();
            dbg!(locales_dir);
        }
        Arc::new(Filesystem::new(locales_dir))
    };

    if let Some(matches) = matches.subcommand_matches("parse-locale") {
        let lang = if let Some(lan) = matches.value_of("lang") {
            if let Ok(l) = Lang::from_str(lan) {
                l
            } else {
                eprintln!(
                    "`{}` is not a valid language",
                    matches.value_of("lang").unwrap_or("")
                );
                return;
            }
        } else {
            Lang::en_us()
        };
        let locale = filesystem_fetcher.fetch_cli(&lang);
        dbg!(locale);
        return;
    }

    if let Some(library_path) = matches.value_of("library") {
        lib_text = fs::read_to_string(&library_path).expect("No library found at that path");
    }

    let mut db = RootDatabase::new(filesystem_fetcher);
    db.set_references(&lib_text).expect("Could not parse JSON");

    if let Some(_) = matches.subcommand_matches("disamb-index") {
        for (tok, ids) in db.inverted_index(()).iter() {
            // if ids.len() > 1 {
            let token = tok.clone();
            let citekeys: Vec<_> = ids.iter().map(|atom| atom.to_string()).collect();
            dbg!((token, citekeys));
            // }
        }
        return;
    }

    let key = matches
        .value_of("key")
        .map(citeproc::Atom::from)
        .unwrap_or("quagmire2018".into());
    let refr = db.reference(key).expect("Citekey not present in library");

    if let Some(csl_path) = matches.value_of("csl") {
        let text = fs::read_to_string(&csl_path).expect("No CSL file found at that path");
        let driver_r: Result<Driver<Pandoc>, _> = Driver::new(&text, db);
        if let Ok(driver) = driver_r {
            // driver.dump_macro("issued-year");
            // driver.dump_ir(&refr);

            let serialized = driver.single(&refr);

            // println!("{}", serialized);

            let header = r#"{"blocks":[{"t":"Para","c":"#;
            let footer = r#"}],"pandoc-api-version":[1,17,5,4],"meta":{}}"#;
            println!("{}{}{}", header, serialized, footer);
        } else if let Err(e) = driver_r {
            citeproc::style::error::file_diagnostics(&e, &csl_path, &text);
        }
    }
}

fn pandoc_meta_str<'a>(doc: &'a PandocDocument, key: &str) -> Option<&'a str> {
    doc.0.lookup(key).and_then(|value| match value {
        // for metadata passed through the command line
        // --metadata csl=my-style.csl
        MetaValue::MetaString(s) => Some(s.as_str()),
        MetaValue::MetaInlines(inlines) => match &inlines[..] {
            // for inline paths with no spaces (otherwise they get split with
            // Inline::Space)
            // csl: "my-style.csl"
            &[Inline::Str(ref s)] => Some(s.as_str()),
            // for inline paths with spaces
            // csl: "`my style.csl`{=raw}"
            &[Inline::RawInline(_, ref s)] => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    })
}

fn do_pandoc() {
    let filter_args = App::new("pandoc_filter")
        .arg(Arg::with_name("output_format").required(false).index(1))
        .get_matches();

    let _output_format = filter_args.value_of("output_format").unwrap_or("none");

    // already BufReader
    let input = std::io::stdin();
    // already LineWriter buffered, but we're only writing one line of JSON so not too bad
    let output = std::io::stdout();

    let mut doc: PandocDocument =
        serde_json::from_reader(input).expect("could not parse pandoc json");

    let filesystem_fetcher = {
        let locales_dir = None
            // TODO: read metadata
            .unwrap_or_else(|| {
                let pd = ProjectDirs::from("net", "cormacrelf", "citeproc-rs")
                    .expect("No home directory found.");
                let mut locales_dir = pd.cache_dir().to_owned();
                locales_dir.push("locales");
                locales_dir
            });
        Arc::new(Filesystem::new(locales_dir))
    };
    let mut db = RootDatabase::new(filesystem_fetcher);

    if let Some(library_path) = pandoc_meta_str(&doc, "bibliography") {
        let lib_text =
            fs::read_to_string(&library_path).expect("No bibliography found at that path");
        db.set_references(&lib_text).expect("Could not parse JSON");
    }

    db.init_clusters(pandoc::get_clusters(&mut doc));

    let csl_path = pandoc_meta_str(&doc, "csl").expect("No csl path provided through metadata");
    let text = fs::read_to_string(&csl_path).expect("No CSL file found at that path");

    let driver_r: Result<Driver<Pandoc>, _> = Driver::new(&text, db);
    if let Ok(driver) = driver_r {
        use citeproc::db::ReferenceDatabase;
        use rayon::prelude::*;
        let ids = driver.db.cluster_ids(());
        ids.par_iter()
            .for_each_with(driver.snap(), |snap, &cluster_id| {
                let _ = snap.0.built_cluster(cluster_id);
            });

        pandoc::write_clusters(&mut doc, &driver.db);

        serde_json::to_writer(output, &doc).expect("could not write pandoc json");
    } else if let Err(e) = driver_r {
        citeproc::style::error::file_diagnostics(&e, &csl_path, &text);
    }
}

pub struct Filesystem {
    root: PathBuf,
}

impl Filesystem {
    pub fn new(repo_dir: impl Into<PathBuf>) -> Self {
        Filesystem {
            root: repo_dir.into(),
        }
    }
}

impl LocaleFetcher for Filesystem {
    fn fetch_string(&self, lang: &Lang) -> Result<String, std::io::Error> {
        let mut path = self.root.clone();
        path.push(&format!("locales-{}.xml", lang));
        fs::read_to_string(path)
    }
}
