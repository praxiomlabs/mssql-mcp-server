//! Security module for query validation and SQL injection prevention.

mod identifiers;
mod injection;
mod validation;

pub use identifiers::{
    escape_identifier, is_reserved_keyword, parse_qualified_name, safe_identifier,
    validate_identifier, validate_not_reserved, warn_if_reserved,
};
pub use injection::InjectionDetector;
pub use validation::{QueryValidator, ValidationMode, ValidationResult};
