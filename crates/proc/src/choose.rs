// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright © 2018 Corporation for Digital Scholarship

use crate::prelude::*;

use crate::ir::ConditionalDisambIR;
use citeproc_io::DateOrRange;
use csl::{
    Choose, Cond, CondSet, Conditions, CslType, Element, Else, IfThen, Match, Position,
};
use csl::{AnyVariable, DateVariable};

use std::sync::{Arc, Mutex};

impl<'c, O, I> Proc<'c, O, I> for Arc<Choose>
where
    O: OutputFormat,
    I: OutputFormat,
{
    fn intermediate(
        &self,
        db: &impl IrDatabase,
        state: &mut IrState,
        ctx: &CiteContext<'c, O, I>,
    ) -> IrSum<O> {
        let make_mutex = |d: bool, content: IR<O>, gv: GroupVars| {
            if d {
                (
                    IR::ConditionalDisamb(Arc::new(Mutex::new(ConditionalDisambIR {
                        choose: self.clone(),
                        done: false,
                        ir: Box::new(content),
                        group_vars: gv,
                    }))),
                    gv,
                )
            } else {
                (content, gv)
            }
        };
        // XXX: should you treat conditional evaluations as a "variable test"?
        let Choose(ref head, ref rest, ref last) = **self;
        let mut disamb = false;
        let mut found;
        {
            let BranchEval {
                disambiguate,
                content,
            } = eval_ifthen(db, head, state, ctx);
            found = content;
            disamb = disamb || disambiguate;
        }
        // check the <if> element
        if let Some((content, gv)) = found {
            return make_mutex(disamb, content, gv);
        } else {
            // check the <else-if> elements
            for branch in rest.iter() {
                if found.is_some() {
                    break;
                }
                let BranchEval {
                    disambiguate,
                    content,
                } = eval_ifthen(db, branch, state, ctx);
                found = content;
                disamb = disamb || disambiguate;
            }
        }
        // did any of the <else-if> elements match?
        if let Some((content, gv)) = found {
            make_mutex(disamb, content, gv)
        } else {
            // if not, <else>
            let Else(ref els) = last;
            let (content, gv) = sequence_basic(db, state, ctx, &els);
            make_mutex(disamb, content, gv)
        }
    }
}

impl Disambiguation<Markup> for Choose {
    fn ref_ir(
        &self,
        db: &impl IrDatabase,
        ctx: &RefContext<Markup>,
        state: &mut IrState,
        stack: Formatting,
    ) -> (RefIR, GroupVars) {
        let Choose(head, rest, last) = self;
        if let Some(els) = eval_ifthen_ref(head, ctx, &mut state.disamb_count).0 {
            return ref_sequence_basic(db, state, ctx, els, stack);
        }
        for branch in rest {
            if let Some(els) = eval_ifthen_ref(branch, ctx, &mut state.disamb_count).0 {
                return ref_sequence_basic(db, state, ctx, els, stack);
            }
        }
        ref_sequence_basic(db, state, ctx, &last.0, stack)
    }
}

struct BranchEval<O: OutputFormat> {
    // the bools indicate if disambiguate was set
    disambiguate: bool,
    content: Option<IrSum<O>>,
}

fn eval_ifthen<'c, O, I>(
    db: &impl IrDatabase,
    branch: &'c IfThen,
    state: &mut IrState,
    ctx: &CiteContext<'c, O, I>,
) -> BranchEval<O>
where
    O: OutputFormat,
    I: OutputFormat,
{
    let IfThen(ref conditions, ref elements) = *branch;
    let (matched, disambiguate) = eval_conditions(conditions, ctx, /* phony, not used */ 0);
    let content = if matched {
        Some(sequence_basic(db, state, ctx, &elements))
    } else {
        None
    };
    BranchEval {
        disambiguate,
        content,
    }
}

fn eval_ifthen_ref<'c, Ck>(
    branch: &'c IfThen,
    checker: &Ck,
    disamb_count: &mut u32,
) -> (Option<&'c [Element]>, bool)
where
    Ck: CondChecker,
{
    let IfThen(ref conditions, ref elements) = *branch;
    let (matched, disambiguate) = eval_conditions(conditions, checker, *disamb_count);
    if disambiguate {
        *disamb_count += 1;
    }
    let content = if matched {
        Some(elements.as_slice())
    } else {
        None
    };
    (content, disambiguate)
}

fn run_matcher<I: Iterator<Item = bool>>(bools: &mut I, match_type: &Match) -> bool {
    match *match_type {
        Match::Any => bools.any(|b| b),
        Match::Nand => bools.any(|b| !b),
        Match::All => bools.all(|b| b),
        Match::None => bools.all(|b| !b),
    }
}

/// first bool is the match result;
/// second bool is disambiguate=true
///
/// Pass current_count = std::u32::MAX if you don't want a RefContext to return true from
/// is_disambiguate()
pub fn eval_conditions<'c, Ck>(
    conditions: &'c Conditions,
    checker: &Ck,
    current_count: u32,
) -> (bool, bool)
where
    Ck: CondChecker,
{
    let Conditions(ref match_type, ref conditions) = *conditions;
    let mut tests = conditions
        .iter()
        .map(|c| eval_condset(c, checker, current_count));
    let disambiguate = conditions.iter().any(|c| {
        c.conds.contains(&Cond::Disambiguate(true)) || c.conds.contains(&Cond::Disambiguate(false))
    });

    (run_matcher(&mut tests, match_type), disambiguate)
}

fn eval_condset<'c, Ck>(cond_set: &'c CondSet, checker: &Ck, current_count: u32) -> bool
where
    Ck: CondChecker,
{
    let features = checker.features();

    let mut iter_all = cond_set.conds.iter().filter_map(|cond| {
        Some(match cond {
            Cond::Variable(var) => checker.has_variable(*var),
            Cond::IsNumeric(var) => checker.is_numeric(*var),
            Cond::Disambiguate(d) => *d == checker.is_disambiguate(current_count),
            Cond::Type(typ) => checker.csl_type() == *typ,
            // None in a bibliography
            Cond::Position(pos) => checker.position().map_or(false, |p| p.matches(*pos)),
            Cond::Locator(typ) => checker.locator_type() == Some(*typ),

            Cond::HasYearOnly(_) | Cond::HasMonthOrSeason(_) | Cond::HasDay(_)
                if !features.condition_date_parts =>
            {
                return None;
            }

            Cond::HasYearOnly(dvar) => checker.has_year_only(*dvar),
            Cond::HasMonthOrSeason(dvar) => checker.has_month_or_season(*dvar),
            Cond::HasDay(dvar) => checker.has_day(*dvar),
            _ => return None,
        })
    });

    run_matcher(&mut iter_all, &cond_set.match_type)
}

use csl::Features;
use csl::LocatorType;

pub struct UselessCondChecker;
impl CondChecker for UselessCondChecker {
    fn has_variable(&self, _var: AnyVariable) -> bool {
        false
    }
    fn is_numeric(&self, _var: AnyVariable) -> bool {
        false
    }
    fn is_disambiguate(&self, _: u32) -> bool {
        false
    }
    fn csl_type(&self) -> CslType {
        CslType::Book
    }
    fn locator_type(&self) -> Option<LocatorType> {
        None
    }
    fn get_date(&self, _dvar: DateVariable) -> Option<&DateOrRange> {
        None
    }
    fn position(&self) -> Option<Position> {
        None
    }
    fn features(&self) -> &csl::version::Features {
        lazy_static::lazy_static! {
            static ref NO_FEATURES: Features = {
                Features::new()
            };
        };
        &*NO_FEATURES
    }
}

pub trait CondChecker {
    fn has_variable(&self, var: AnyVariable) -> bool;
    fn is_numeric(&self, var: AnyVariable) -> bool;
    /// Count is for references only, so IRs can slowly increase the disamb count and incrementally
    /// enable disambiguate="true" (not technically part of the spec, but seems worthwhile); see
    /// disambiguate_IncrementalExtraText.txt
    fn is_disambiguate(&self, current_count: u32) -> bool;
    fn csl_type(&self) -> CslType;
    fn locator_type(&self) -> Option<LocatorType>;
    fn get_date(&self, dvar: DateVariable) -> Option<&DateOrRange>;
    fn position(&self) -> Option<Position>;
    fn features(&self) -> &Features;
    fn has_year_only(&self, dvar: DateVariable) -> bool {
        self.get_date(dvar)
            .map(|dor| match dor {
                DateOrRange::Single(d) => d.month == 0 && d.day == 0,
                DateOrRange::Range(d1, d2) => {
                    d1.month == 0 && d1.day == 0 && d2.month == 0 && d2.day == 0
                }
                _ => false,
            })
            .unwrap_or(false)
    }
    fn has_month_or_season(&self, dvar: DateVariable) -> bool {
        self.get_date(dvar)
            .map(|dor| match dor {
                DateOrRange::Single(d) => d.month != 0,
                DateOrRange::Range(d1, d2) => {
                    // XXX: is OR the right operator here?
                    d1.month != 0 || d2.month != 0
                }
                _ => false,
            })
            .unwrap_or(false)
    }
    fn has_day(&self, dvar: DateVariable) -> bool {
        self.get_date(dvar)
            .map(|dor| match dor {
                DateOrRange::Single(d) => d.day != 0,
                DateOrRange::Range(d1, d2) => {
                    // XXX: is OR the right operator here?
                    d1.day != 0 || d2.day != 0
                }
                _ => false,
            })
            .unwrap_or(false)
    }
    // TODO: is_uncertain_date ("ca. 2003"). CSL and CSL-JSON do not specify how this is meant to
    // work.
    // Actually, is_uncertain_date (+ circa) is is a CSL-JSON thing.
}
