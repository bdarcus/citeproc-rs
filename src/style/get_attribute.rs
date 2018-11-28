use std::str::FromStr;
use crate::style::error::*;
use strum::EnumProperty;
use std::convert::AsRef;
use roxmltree::{ Node };

pub const CSL_VERSION: CslVersion = CslVersion::Csl101;

pub trait GetAttribute where Self : Sized {
    fn get_attr(s: &str, csl_version: CslVersion) -> Result<Self, UnknownAttributeValue>;
}

#[derive(AsRefStr, EnumString, Debug, PartialEq, Eq)]
#[strum(serialize_all="snake_case")]
pub enum CslVersion {
    #[strum(serialize = "csl101")]
    Csl101,
    #[strum(serialize = "cslM")]
    CslM,
}

impl CslVersion {
    fn filter_arg<T : EnumProperty>(&self, val: T) -> Option<T> {
            let version: &str = self.as_ref();
            if let Some("0") = val.get_str(version) {
                return None;
            }
            Some(val)
        }
}

impl<T: FromStr + EnumProperty> GetAttribute for T {
    fn get_attr(s: &str, csl_version: CslVersion) -> Result<Self, UnknownAttributeValue> {
        match T::from_str(s) {
            Ok(a) => csl_version
                .filter_arg(a)
                .ok_or_else(|| UnknownAttributeValue::new(s)),
            Err(_) => Err(UnknownAttributeValue::new(s))
        }
    }
}

pub fn attribute_bool(node: &Node, attr: &str, default: bool) -> Result<bool, InvalidCsl> {
    match node.attribute(attr) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        None => Ok(default),
        Some(s) => Err(InvalidCsl::attr_val(node, attr, s))?
    }
}

pub fn attribute_only_true(node: &Node, attr: &str) -> Result<bool, InvalidCsl> {
    match node.attribute(attr) {
        Some("true") => Ok(true),
        None => Ok(false),
        Some(s) => Err(InvalidCsl::attr_val(node, attr, s))
    }
}

pub fn attribute_int(node: &Node, attr: &str, default: u32) -> Result<u32, InvalidCsl> {
    match node.attribute(attr) {
        Some(s) => {
            let parsed = u32::from_str_radix(s, 10);
            parsed.map_err(|e| InvalidCsl::bad_int(node, attr, e))
        },
        None => Ok(default),
    }
}

pub fn attribute_string(node: &Node, attr: &str) -> String {
    node.attribute(attr).map(String::from).unwrap_or_else(|| String::from(""))
}

pub fn attribute_required<T: GetAttribute>(node: &Node, attr: &str) -> Result<T, InvalidCsl> {
    match node.attribute(attr) {
        Some(a) => match T::get_attr(a, CSL_VERSION) {
            Ok(val) => Ok(val),
            Err(e)  => Err(InvalidCsl::attr_val(node, attr, &e.value))
        },
        None => Err(InvalidCsl::new(node, format!("Must have '{}' attribute", attr)))
    }
}

pub fn attribute_optional<T: Default + GetAttribute>(node: &Node, attr: &str) -> Result<T, InvalidCsl> {
    match node.attribute(attr) {
        Some(a) => match T::get_attr(a, CSL_VERSION) {
            Ok(val) => Ok(val),
            Err(e)  => Err(InvalidCsl::attr_val(node, attr, &e.value))
        },
        None => Ok(T::default())
    }
}

pub fn attribute_array<T: GetAttribute>(node: &Node, attr: &str) -> Result<Vec<T>, InvalidCsl> {
    match node.attribute(attr) {
        Some(a) => {
            let split: Result<Vec<_>, _> = a.split(" ")
                .map(|a| T::get_attr(a, CSL_VERSION))
                .collect();
            match split {
                Ok(val) => Ok(val),
                Err(e)  => Err(InvalidCsl::attr_val(node, attr, &e.value))
            }
        },
        None => Ok(vec![])
    }
}

pub fn attribute_optional2<T : Default, F : FnOnce(&str) -> Result<T, UnknownAttributeValue>>(node: &Node, attr: &str, result: F) -> Result<T, InvalidCsl>
{
    match node.attribute(attr) {
        Some(a) => match result(a) {
            Ok(val) => Ok(val),
            Err(e)  => Err(InvalidCsl::attr_val(node, attr, &e.value))
        },
        None => Ok(T::default())
    }
}

