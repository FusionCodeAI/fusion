//! Pure-Rust JSON Schema validator, tool call argument validator,
//! argument repair & coercion engine, and schema template generator.
//!
//! Provides comprehensive JSON Schema validation supporting Draft-04, Draft-07,
//! and 2020-12 specifications commonly used in tool definitions and function calling:
//! - **Type Checking**: Strict type checking for `string`, `number`, `integer`, `boolean`,
//!   `null`, `object`, `array`, and multi-type arrays (e.g. `["string", "null"]`).
//! - **Object Validation**: `properties`, `required`, `additionalProperties` (bool or schema),
//!   `patternProperties`, `propertyNames`, `minProperties`, `maxProperties`, `dependentRequired`,
//!   and `dependentSchemas`.
//! - **Array Validation**: `items` (single schema or tuple arrays), `prefixItems`, `additionalItems`,
//!   `minItems`, `maxItems`, `uniqueItems` (deep equality), and `contains` (`minContains`/`maxContains`).
//! - **Number & Range Validation**: `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`,
//!   and `multipleOf` (with float epsilon handling).
//! - **String & Format Validation**: `minLength`, `maxLength`, `pattern` (Regex matching),
//!   and standard string formats: `email`, `uri`, `uri-reference`, `ipv4`, `ipv6`, `date-time`,
//!   `date`, `time`, `uuid`, `hostname`, `regex`, and `json-pointer`.
//! - **Combinators & Conditionals**: `allOf`, `anyOf`, `oneOf`, `not`, and `if`/`then`/`else`.
//! - **Schema References**: `$ref` pointer resolution (`#/definitions/...`, `#/$defs/...`, `#/properties/...`,
//!   and root pointers with RFC 6901 `~0` and `~1` unescaping) with cycle protection.
//! - **Tool Call Argument Repair & Coercion**: Coerces stringified numbers/booleans, parses JSON strings,
//!   applies schema defaults, strips unknown properties, and provides fuzzy typo suggestions for unknown fields.
//! - **Template & Scaffold Generation**: Generates example JSON data from schemas with comments and sample values.
//! - **Schema Documentation & Diffing**: Generates clean Markdown documentation tables and detects breaking changes.

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use crate::provider::types::ToolDefinition;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Diagnostic Data Models: Errors, Warnings, and Validation Reports
// ===========================================================================

/// A specific validation error identifying where and why schema validation failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationError {
    /// RFC 6901 JSON pointer to the failing instance location (e.g. `"/users/0/email"` or `""` for root).
    pub instance_path: String,
    /// JSON pointer into the schema that triggered the failure (e.g. `"/properties/email/format"`).
    pub schema_path: String,
    /// The specific JSON Schema keyword that failed (e.g. `"type"`, `"required"`, `"minimum"`).
    pub keyword: String,
    /// Human-readable explanation of the validation failure.
    pub message: String,
    /// Expected schema requirement or value description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    /// Actual received value or description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
    /// Actionable suggestion for fixing the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl ValidationError {
    pub fn new(
        instance_path: impl Into<String>,
        schema_path: impl Into<String>,
        keyword: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            instance_path: instance_path.into(),
            schema_path: schema_path.into(),
            keyword: keyword.into(),
            message: message.into(),
            expected: None,
            actual: None,
            suggestion: None,
        }
    }

    pub fn with_expected(mut self, expected: impl Into<Value>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_actual(mut self, actual: impl Into<Value>) -> Self {
        self.actual = Some(actual.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// A non-fatal validation warning (e.g. deprecated property or coercible type mismatch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub instance_path: String,
    pub message: String,
    pub code: String,
}

/// Comprehensive report summarizing validation results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// True if the instance satisfies all schema requirements.
    pub valid: bool,
    /// List of validation errors encountered.
    pub errors: Vec<ValidationError>,
    /// List of non-fatal warnings.
    pub warnings: Vec<ValidationWarning>,
    /// Total error count.
    pub error_count: usize,
    /// Total warning count.
    pub warning_count: usize,
}

impl ValidationReport {
    pub fn ok() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            error_count: 0,
            warning_count: 0,
        }
    }

    pub fn from_errors(errors: Vec<ValidationError>) -> Self {
        let valid = errors.is_empty();
        let error_count = errors.len();
        Self {
            valid,
            errors,
            warnings: Vec::new(),
            error_count,
            warning_count: 0,
        }
    }

    pub fn add_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
        self.warning_count = self.warnings.len();
    }

    /// Formats a human-readable diagnostic summary.
    pub fn format_pretty(&self) -> String {
        if self.valid {
            if self.warnings.is_empty() {
                return "✓ JSON instance is valid against schema.".to_string();
            } else {
                let mut out = format!(
                    "✓ JSON instance is valid (with {} warnings):\n",
                    self.warning_count
                );
                for (i, w) in self.warnings.iter().enumerate() {
                    out.push_str(&format!(
                        "  [{}] Warning at `{}`: {}\n",
                        i + 1,
                        w.instance_path,
                        w.message
                    ));
                }
                return out;
            }
        }

        let mut out = format!(
            "✗ JSON validation failed with {} error(s):\n",
            self.error_count
        );
        for (i, err) in self.errors.iter().enumerate() {
            let path_display = if err.instance_path.is_empty() {
                "<root>".to_string()
            } else {
                err.instance_path.clone()
            };
            out.push_str(&format!(
                "  [{}] Error at `{}` (keyword: `{}`):\n      {}\n",
                i + 1,
                path_display,
                err.keyword,
                err.message
            ));
            if let Some(expected) = &err.expected {
                out.push_str(&format!("      Expected: {}\n", expected));
            }
            if let Some(actual) = &err.actual {
                out.push_str(&format!("      Actual:   {}\n", actual));
            }
            if let Some(suggestion) = &err.suggestion {
                out.push_str(&format!("      💡 Suggestion: {}\n", suggestion));
            }
        }
        out
    }
}

// ===========================================================================
// Levenshtein & Fuzzy Name Matching
// ===========================================================================

/// Calculates the Levenshtein edit distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[m][n]
}

/// Finds the closest candidate string to `target` among `candidates`.
pub fn find_closest_match<'a>(target: &str, candidates: &'a [String]) -> Option<&'a str> {
    let target_norm = target.to_lowercase().replace('-', "_");
    let mut best_match: Option<&'a str> = None;
    let mut best_dist = usize::MAX;

    for candidate in candidates {
        let cand_norm = candidate.to_lowercase().replace('-', "_");
        if target_norm == cand_norm {
            return Some(candidate);
        }
        let dist = levenshtein_distance(&target_norm, &cand_norm);
        if dist < best_dist && dist <= 3 {
            best_dist = dist;
            best_match = Some(candidate);
        }
    }

    best_match
}

// ===========================================================================
// JSON Pointer Resolution (RFC 6901)
// ===========================================================================

/// Unescapes RFC 6901 JSON Pointer tokens (`~1` -> `/`, `~0` -> `~`).
pub fn unescape_json_pointer_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Escapes a string for use in an RFC 6901 JSON Pointer token (`~` -> `~0`, `/` -> `~1`).
pub fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Resolves a JSON Pointer against a root JSON value.
pub fn resolve_json_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let pointer = pointer.strip_prefix('#').unwrap_or(pointer);
    if pointer.is_empty() || pointer == "/" {
        return Some(root);
    }

    let trimmed = pointer.strip_prefix('/').unwrap_or(pointer);
    let mut current = root;

    for token in trimmed.split('/') {
        let unescaped = unescape_json_pointer_token(token);
        match current {
            Value::Object(map) => {
                current = map.get(&unescaped)?;
            }
            Value::Array(arr) => {
                if let Ok(idx) = unescaped.parse::<usize>() {
                    current = arr.get(idx)?;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    Some(current)
}

// ===========================================================================
// Format Validators
// ===========================================================================

/// Validates standard string formats.
pub fn validate_string_format(format: &str, value: &str) -> bool {
    match format {
        "email" | "idn-email" => {
            if value.is_empty() || value.len() > 254 {
                return false;
            }
            if let Some((local, domain)) = value.split_once('@') {
                !local.is_empty()
                    && !domain.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && !local.contains(' ')
                    && !domain.contains(' ')
            } else {
                false
            }
        }
        "ipv4" => {
            let parts: Vec<&str> = value.split('.').collect();
            if parts.len() != 4 {
                return false;
            }
            parts.iter().all(|p| {
                if p.is_empty() || (p.len() > 1 && p.starts_with('0')) {
                    false
                } else {
                    p.parse::<u8>().is_ok()
                }
            })
        }
        "ipv6" => value.parse::<std::net::Ipv6Addr>().is_ok(),
        "uuid" => {
            if value.len() != 36 {
                return false;
            }
            let parts: Vec<&str> = value.split('-').collect();
            if parts.len() != 5 {
                return false;
            }
            let lengths = [8, 4, 4, 4, 12];
            for (part, &expected_len) in parts.iter().zip(lengths.iter()) {
                if part.len() != expected_len || !part.chars().all(|c| c.is_ascii_hexdigit()) {
                    return false;
                }
            }
            true
        }
        "date-time" => {
            // RFC 3339 format validation: YYYY-MM-DDTHH:MM:SS(Z|+HH:MM|-HH:MM)
            chrono::DateTime::parse_from_rfc3339(value).is_ok()
        }
        "date" => {
            // YYYY-MM-DD
            if value.len() != 10 {
                return false;
            }
            let parts: Vec<&str> = value.split('-').collect();
            if parts.len() != 3 {
                return false;
            }
            if let (Ok(y), Ok(m), Ok(d)) = (
                parts[0].parse::<i32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
            ) {
                y >= 1000 && y <= 9999 && m >= 1 && m <= 12 && d >= 1 && d <= 31
            } else {
                false
            }
        }
        "time" => {
            // HH:MM:SS
            let time_part = value.split(['Z', '+', '-']).next().unwrap_or(value);
            let parts: Vec<&str> = time_part.split(':').collect();
            if parts.len() != 3 {
                return false;
            }
            if let (Ok(h), Ok(m), Ok(s)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].split('.').next().unwrap_or("").parse::<u32>(),
            ) {
                h < 24 && m < 60 && s < 60
            } else {
                false
            }
        }
        "uri" => {
            // Must have a scheme and not be empty
            if let Some(idx) = value.find(':') {
                idx > 0
                    && value[..idx]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            } else {
                false
            }
        }
        "uri-reference" | "iri" | "iri-reference" => !value.contains(' '),
        "hostname" | "idn-hostname" => {
            if value.is_empty() || value.len() > 253 {
                return false;
            }
            value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        }
        "regex" => Regex::new(value).is_ok(),
        "json-pointer" => value.is_empty() || value.starts_with('/'),
        _ => true, // Unknown or unsupported format keywords are ignored per JSON Schema spec
    }
}

// ===========================================================================
// Core JSON Schema Validator Engine
// ===========================================================================

/// Pure-Rust JSON Schema Validator engine with compiled regex cache and `$ref` resolution.
pub struct JsonSchemaValidator {
    root_schema: Value,
    regex_cache: RwLock<HashMap<String, Result<Regex, String>>>,
    max_depth: usize,
}

impl JsonSchemaValidator {
    /// Creates a new validator for the given root JSON Schema.
    pub fn new(root_schema: Value) -> Self {
        Self {
            root_schema,
            regex_cache: RwLock::new(HashMap::new()),
            max_depth: 64,
        }
    }

    /// Sets the maximum recursive evaluation depth (default 64).
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Returns a reference to the root schema.
    pub fn root_schema(&self) -> &Value {
        &self.root_schema
    }

    /// Compiles or retrieves a cached regex.
    fn get_regex(&self, pattern: &str) -> Result<Regex, String> {
        {
            let cache = self.regex_cache.read().unwrap();
            if let Some(res) = cache.get(pattern) {
                return res.clone();
            }
        }
        let compiled = Regex::new(pattern).map_err(|e| e.to_string());
        let mut cache = self.regex_cache.write().unwrap();
        cache.insert(pattern.to_string(), compiled.clone());
        compiled
    }

    /// Validates an instance JSON Value against the root schema.
    pub fn validate(&self, instance: &Value) -> ValidationReport {
        let mut errors = Vec::new();
        let mut visited_refs = HashSet::new();
        self.validate_node(
            &self.root_schema,
            instance,
            "",
            "",
            0,
            &mut visited_refs,
            &mut errors,
        );
        ValidationReport::from_errors(errors)
    }

    /// Internal recursive validator node.
    fn validate_node(
        &self,
        schema: &Value,
        instance: &Value,
        instance_path: &str,
        schema_path: &str,
        depth: usize,
        visited_refs: &mut HashSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if depth > self.max_depth {
            errors.push(ValidationError::new(
                instance_path,
                schema_path,
                "$depth",
                format!(
                    "Maximum schema evaluation depth of {} exceeded (possible cyclic $ref).",
                    self.max_depth
                ),
            ));
            return;
        }

        // Schema boolean: true = accept all, false = reject all
        if let Value::Bool(b) = schema {
            if !*b {
                errors.push(ValidationError::new(
                    instance_path,
                    schema_path,
                    "false",
                    "Schema is boolean `false` (rejects all values).",
                ));
            }
            return;
        }

        let schema_obj = match schema {
            Value::Object(map) => map,
            _ => return, // Non-object non-bool schemas are treated as true
        };

        // 1. `$ref` Reference Resolution
        if let Some(Value::String(ref_uri)) = schema_obj.get("$ref") {
            let ref_key = format!("{}:{}", ref_uri, instance_path);
            if visited_refs.contains(&ref_key) {
                // Detected recursion cycle for the same instance path
                return;
            }
            visited_refs.insert(ref_key.clone());

            if let Some(target_schema) = resolve_json_pointer(&self.root_schema, ref_uri) {
                let next_schema_path = format!("{}/$ref", schema_path);
                self.validate_node(
                    target_schema,
                    instance,
                    instance_path,
                    &next_schema_path,
                    depth + 1,
                    visited_refs,
                    errors,
                );
            } else {
                errors.push(ValidationError::new(
                    instance_path,
                    schema_path,
                    "$ref",
                    format!("Could not resolve $ref pointer `{}`.", ref_uri),
                ));
            }

            visited_refs.remove(&ref_key);
            return;
        }

        // 2. Type Checking (`type`)
        if let Some(type_val) = schema_obj.get("type") {
            let matches_type = match type_val {
                Value::String(t) => self.check_type(t, instance),
                Value::Array(types) => types.iter().any(|t| {
                    if let Value::String(ts) = t {
                        self.check_type(ts, instance)
                    } else {
                        false
                    }
                }),
                _ => true,
            };

            if !matches_type {
                let actual_type_name = self.get_value_type_name(instance);
                let expected_desc = match type_val {
                    Value::String(s) => s.clone(),
                    Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                    _ => type_val.to_string(),
                };
                errors.push(
                    ValidationError::new(
                        instance_path,
                        format!("{}/type", schema_path),
                        "type",
                        format!(
                            "Expected type `{}`, but got `{}`.",
                            expected_desc, actual_type_name
                        ),
                    )
                    .with_expected(type_val.clone())
                    .with_actual(json!(actual_type_name))
                    .with_suggestion(format!(
                        "Provide a value of type `{}` instead.",
                        expected_desc
                    )),
                );
                return; // Early return on type mismatch for this branch
            }
        }

        // 3. Enum & Const
        if let Some(Value::Array(allowed_values)) = schema_obj.get("enum") {
            if !allowed_values.contains(instance) {
                let options_str = allowed_values
                    .iter()
                    .take(10)
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(
                    ValidationError::new(
                        instance_path,
                        format!("{}/enum", schema_path),
                        "enum",
                        format!("Value is not in allowed enum list: [{}]", options_str),
                    )
                    .with_expected(json!(allowed_values))
                    .with_actual(instance.clone())
                    .with_suggestion(format!("Choose one of: [{}]", options_str)),
                );
            }
        }

        if let Some(const_val) = schema_obj.get("const") {
            if const_val != instance {
                errors.push(
                    ValidationError::new(
                        instance_path,
                        format!("{}/const", schema_path),
                        "const",
                        format!("Value must exactly match const value `{}`.", const_val),
                    )
                    .with_expected(const_val.clone())
                    .with_actual(instance.clone()),
                );
            }
        }

        // 4. Number / Integer Constraints
        if let Some(num) = instance.as_f64() {
            if let Some(min_val) = schema_obj.get("minimum").and_then(|v| v.as_f64()) {
                let exclusive = schema_obj
                    .get("exclusiveMinimum")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if exclusive {
                    if num <= min_val {
                        errors.push(ValidationError::new(
                            instance_path,
                            format!("{}/minimum", schema_path),
                            "exclusiveMinimum",
                            format!("Value {} must be strictly greater than {}.", num, min_val),
                        ));
                    }
                } else if num < min_val {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/minimum", schema_path),
                        "minimum",
                        format!(
                            "Value {} must be greater than or equal to {}.",
                            num, min_val
                        ),
                    ));
                }
            }

            // Draft-07 numeric exclusiveMinimum
            if let Some(excl_min) = schema_obj.get("exclusiveMinimum").and_then(|v| v.as_f64()) {
                if num <= excl_min {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/exclusiveMinimum", schema_path),
                        "exclusiveMinimum",
                        format!("Value {} must be strictly greater than {}.", num, excl_min),
                    ));
                }
            }

            if let Some(max_val) = schema_obj.get("maximum").and_then(|v| v.as_f64()) {
                let exclusive = schema_obj
                    .get("exclusiveMaximum")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if exclusive {
                    if num >= max_val {
                        errors.push(ValidationError::new(
                            instance_path,
                            format!("{}/maximum", schema_path),
                            "exclusiveMaximum",
                            format!("Value {} must be strictly less than {}.", num, max_val),
                        ));
                    }
                } else if num > max_val {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/maximum", schema_path),
                        "maximum",
                        format!("Value {} must be less than or equal to {}.", num, max_val),
                    ));
                }
            }

            // Draft-07 numeric exclusiveMaximum
            if let Some(excl_max) = schema_obj.get("exclusiveMaximum").and_then(|v| v.as_f64()) {
                if num >= excl_max {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/exclusiveMaximum", schema_path),
                        "exclusiveMaximum",
                        format!("Value {} must be strictly less than {}.", num, excl_max),
                    ));
                }
            }

            if let Some(mult) = schema_obj.get("multipleOf").and_then(|v| v.as_f64()) {
                if mult > 0.0 {
                    let rem = (num / mult).round() * mult - num;
                    if rem.abs() > 1e-9 {
                        errors.push(ValidationError::new(
                            instance_path,
                            format!("{}/multipleOf", schema_path),
                            "multipleOf",
                            format!("Value {} is not a multiple of {}.", num, mult),
                        ));
                    }
                }
            }
        }

        // 5. String Constraints
        if let Some(s) = instance.as_str() {
            let char_count = s.chars().count();
            if let Some(min_len) = schema_obj.get("minLength").and_then(|v| v.as_u64()) {
                if char_count < min_len as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/minLength", schema_path),
                        "minLength",
                        format!(
                            "String length {} is shorter than minimum length {}.",
                            char_count, min_len
                        ),
                    ));
                }
            }

            if let Some(max_len) = schema_obj.get("maxLength").and_then(|v| v.as_u64()) {
                if char_count > max_len as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/maxLength", schema_path),
                        "maxLength",
                        format!(
                            "String length {} is longer than maximum length {}.",
                            char_count, max_len
                        ),
                    ));
                }
            }

            if let Some(Value::String(pat)) = schema_obj.get("pattern") {
                match self.get_regex(pat) {
                    Ok(re) => {
                        if !re.is_match(s) {
                            errors.push(
                                ValidationError::new(
                                    instance_path,
                                    format!("{}/pattern", schema_path),
                                    "pattern",
                                    format!(
                                        "String does not match required regex pattern `{}`.",
                                        pat
                                    ),
                                )
                                .with_expected(json!(pat))
                                .with_actual(json!(s)),
                            );
                        }
                    }
                    Err(e) => {
                        errors.push(ValidationError::new(
                            instance_path,
                            format!("{}/pattern", schema_path),
                            "pattern",
                            format!("Invalid regex pattern in schema `{}`: {}", pat, e),
                        ));
                    }
                }
            }

            if let Some(Value::String(fmt)) = schema_obj.get("format") {
                if !validate_string_format(fmt, s) {
                    errors.push(
                        ValidationError::new(
                            instance_path,
                            format!("{}/format", schema_path),
                            "format",
                            format!("String does not match format `{}`: `{}`.", fmt, s),
                        )
                        .with_expected(json!(fmt))
                        .with_actual(json!(s))
                        .with_suggestion(format!(
                            "Ensure the string conforms to standard `{}` format.",
                            fmt
                        )),
                    );
                }
            }
        }

        // 6. Array Constraints
        if let Some(arr) = instance.as_array() {
            if let Some(min_items) = schema_obj.get("minItems").and_then(|v| v.as_u64()) {
                if arr.len() < min_items as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/minItems", schema_path),
                        "minItems",
                        format!(
                            "Array contains {} items, but minimum required is {}.",
                            arr.len(),
                            min_items
                        ),
                    ));
                }
            }

            if let Some(max_items) = schema_obj.get("maxItems").and_then(|v| v.as_u64()) {
                if arr.len() > max_items as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/maxItems", schema_path),
                        "maxItems",
                        format!(
                            "Array contains {} items, exceeding maximum allowed of {}.",
                            arr.len(),
                            max_items
                        ),
                    ));
                }
            }

            if let Some(unique) = schema_obj.get("uniqueItems").and_then(|v| v.as_bool()) {
                if unique {
                    for i in 0..arr.len() {
                        for j in (i + 1)..arr.len() {
                            if arr[i] == arr[j] {
                                errors.push(ValidationError::new(
                                    instance_path,
                                    format!("{}/uniqueItems", schema_path),
                                    "uniqueItems",
                                    format!("Array items must be unique, but item [{}] is identical to item [{}]: {}.", i, j, arr[i]),
                                ));
                                break;
                            }
                        }
                    }
                }
            }

            // `items` or `prefixItems` validation
            let prefix_items = schema_obj
                .get("prefixItems")
                .and_then(|v| v.as_array())
                .or_else(|| {
                    schema_obj.get("items").and_then(|v| match v {
                        Value::Array(items_arr) => Some(items_arr),
                        _ => None,
                    })
                });

            if let Some(tuple_schemas) = prefix_items {
                for (idx, item) in arr.iter().enumerate() {
                    let next_inst_path = format!("{}/{}", instance_path, idx);
                    if idx < tuple_schemas.len() {
                        let next_schema_path = format!("{}/items/{}", schema_path, idx);
                        self.validate_node(
                            &tuple_schemas[idx],
                            item,
                            &next_inst_path,
                            &next_schema_path,
                            depth + 1,
                            visited_refs,
                            errors,
                        );
                    } else if let Some(additional) = schema_obj.get("additionalItems") {
                        let next_schema_path = format!("{}/additionalItems", schema_path);
                        self.validate_node(
                            additional,
                            item,
                            &next_inst_path,
                            &next_schema_path,
                            depth + 1,
                            visited_refs,
                            errors,
                        );
                    }
                }
            } else if let Some(item_schema) = schema_obj.get("items") {
                if item_schema.is_object() || item_schema.is_boolean() {
                    for (idx, item) in arr.iter().enumerate() {
                        let next_inst_path = format!("{}/{}", instance_path, idx);
                        let next_schema_path = format!("{}/items", schema_path);
                        self.validate_node(
                            item_schema,
                            item,
                            &next_inst_path,
                            &next_schema_path,
                            depth + 1,
                            visited_refs,
                            errors,
                        );
                    }
                }
            }

            // `contains`, `minContains`, `maxContains`
            if let Some(contains_schema) = schema_obj.get("contains") {
                let mut match_count = 0usize;
                for item in arr {
                    let mut sub_errs = Vec::new();
                    self.validate_node(
                        contains_schema,
                        item,
                        instance_path,
                        &format!("{}/contains", schema_path),
                        depth + 1,
                        visited_refs,
                        &mut sub_errs,
                    );
                    if sub_errs.is_empty() {
                        match_count += 1;
                    }
                }

                let min_contains = schema_obj
                    .get("minContains")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;

                if match_count < min_contains {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/contains", schema_path),
                        "contains",
                        format!(
                            "Array must contain at least {} matching item(s), but matched {}.",
                            min_contains, match_count
                        ),
                    ));
                }

                if let Some(max_contains) = schema_obj.get("maxContains").and_then(|v| v.as_u64()) {
                    if match_count > max_contains as usize {
                        errors.push(ValidationError::new(
                            instance_path,
                            format!("{}/maxContains", schema_path),
                            "maxContains",
                            format!(
                                "Array must contain at most {} matching item(s), but matched {}.",
                                max_contains, match_count
                            ),
                        ));
                    }
                }
            }
        }

        // 7. Object Constraints
        if let Some(obj) = instance.as_object() {
            // Required properties
            if let Some(Value::Array(req_arr)) = schema_obj.get("required") {
                let declared_props = schema_obj
                    .get("properties")
                    .and_then(|v| v.as_object())
                    .map(|m| m.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();

                for req_val in req_arr {
                    if let Some(req_name) = req_val.as_str() {
                        if !obj.contains_key(req_name) {
                            let mut err = ValidationError::new(
                                instance_path,
                                format!("{}/required", schema_path),
                                "required",
                                format!("Missing required property `{}`.", req_name),
                            )
                            .with_expected(json!(req_name));

                            // Suggest closest match among received keys
                            let received_keys: Vec<String> = obj.keys().cloned().collect();
                            if let Some(suggestion) = find_closest_match(req_name, &received_keys) {
                                err =
                                    err.with_suggestion(format!("Did you mean `{}`?", suggestion));
                            } else if declared_props.contains(&req_name.to_string()) {
                                err = err.with_suggestion(format!(
                                    "Provide the `{}` field in the request object.",
                                    req_name
                                ));
                            }
                            errors.push(err);
                        }
                    }
                }
            }

            // Min/Max Properties
            if let Some(min_props) = schema_obj.get("minProperties").and_then(|v| v.as_u64()) {
                if obj.len() < min_props as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/minProperties", schema_path),
                        "minProperties",
                        format!(
                            "Object contains {} properties, fewer than minProperties of {}.",
                            obj.len(),
                            min_props
                        ),
                    ));
                }
            }

            if let Some(max_props) = schema_obj.get("maxProperties").and_then(|v| v.as_u64()) {
                if obj.len() > max_props as usize {
                    errors.push(ValidationError::new(
                        instance_path,
                        format!("{}/maxProperties", schema_path),
                        "maxProperties",
                        format!(
                            "Object contains {} properties, exceeding maxProperties of {}.",
                            obj.len(),
                            max_props
                        ),
                    ));
                }
            }

            // Property Names validation
            if let Some(prop_names_schema) = schema_obj.get("propertyNames") {
                for key in obj.keys() {
                    let key_val = Value::String(key.clone());
                    let mut name_errors = Vec::new();
                    self.validate_node(
                        prop_names_schema,
                        &key_val,
                        instance_path,
                        &format!("{}/propertyNames", schema_path),
                        depth + 1,
                        visited_refs,
                        &mut name_errors,
                    );
                    if !name_errors.is_empty() {
                        errors.push(ValidationError::new(
                            format!("{}/{}", instance_path, escape_json_pointer_token(key)),
                            format!("{}/propertyNames", schema_path),
                            "propertyNames",
                            format!(
                                "Property name `{}` is invalid according to propertyNames schema.",
                                key
                            ),
                        ));
                    }
                }
            }

            // Properties & PatternProperties & AdditionalProperties
            let declared_properties = schema_obj.get("properties").and_then(|v| v.as_object());
            let pattern_properties = schema_obj
                .get("patternProperties")
                .and_then(|v| v.as_object());
            let additional_props = schema_obj.get("additionalProperties");

            for (key, val) in obj {
                let mut matched_rule = false;
                let next_inst_path = if instance_path.is_empty() {
                    format!("/{}", escape_json_pointer_token(key))
                } else {
                    format!("{}/{}", instance_path, escape_json_pointer_token(key))
                };

                // Explicit declared property
                if let Some(props_map) = declared_properties {
                    if let Some(prop_schema) = props_map.get(key) {
                        matched_rule = true;
                        let next_schema_path = format!(
                            "{}/properties/{}",
                            schema_path,
                            escape_json_pointer_token(key)
                        );
                        self.validate_node(
                            prop_schema,
                            val,
                            &next_inst_path,
                            &next_schema_path,
                            depth + 1,
                            visited_refs,
                            errors,
                        );
                    }
                }

                // Pattern properties
                if let Some(pattern_map) = pattern_properties {
                    for (pat, sub_schema) in pattern_map {
                        match self.get_regex(pat) {
                            Ok(re) => {
                                if re.is_match(key) {
                                    matched_rule = true;
                                    let next_schema_path = format!(
                                        "{}/patternProperties/{}",
                                        schema_path,
                                        escape_json_pointer_token(pat)
                                    );
                                    self.validate_node(
                                        sub_schema,
                                        val,
                                        &next_inst_path,
                                        &next_schema_path,
                                        depth + 1,
                                        visited_refs,
                                        errors,
                                    );
                                }
                            }
                            Err(_) => {}
                        }
                    }
                }

                // Additional properties
                if !matched_rule {
                    if let Some(additional) = additional_props {
                        match additional {
                            Value::Bool(false) => {
                                let mut err = ValidationError::new(
                                    &next_inst_path,
                                    format!("{}/additionalProperties", schema_path),
                                    "additionalProperties",
                                    format!("Unrecognized property `{}` is not allowed (additionalProperties: false).", key),
                                )
                                .with_actual(json!(key));

                                if let Some(props_map) = declared_properties {
                                    let known_keys: Vec<String> =
                                        props_map.keys().cloned().collect();
                                    if let Some(closest) = find_closest_match(key, &known_keys) {
                                        err = err.with_suggestion(format!(
                                            "Did you mean `{}`?",
                                            closest
                                        ));
                                    }
                                }
                                errors.push(err);
                            }
                            Value::Object(_) | Value::Bool(true) => {
                                let next_schema_path =
                                    format!("{}/additionalProperties", schema_path);
                                self.validate_node(
                                    additional,
                                    val,
                                    &next_inst_path,
                                    &next_schema_path,
                                    depth + 1,
                                    visited_refs,
                                    errors,
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Dependent Required / Dependencies
            if let Some(Value::Object(deps_map)) = schema_obj
                .get("dependentRequired")
                .or_else(|| schema_obj.get("dependencies"))
            {
                for (prop_key, dep_val) in deps_map {
                    if obj.contains_key(prop_key) {
                        if let Value::Array(req_keys) = dep_val {
                            for dep_key_val in req_keys {
                                if let Some(dep_key) = dep_key_val.as_str() {
                                    if !obj.contains_key(dep_key) {
                                        errors.push(ValidationError::new(
                                            instance_path,
                                            format!("{}/dependentRequired/{}", schema_path, prop_key),
                                            "dependentRequired",
                                            format!("Property `{}` requires property `{}` to also be present.", prop_key, dep_key),
                                        ));
                                    }
                                }
                            }
                        } else if dep_val.is_object() || dep_val.is_boolean() {
                            // Dependent Schema
                            let next_schema_path =
                                format!("{}/dependentSchemas/{}", schema_path, prop_key);
                            self.validate_node(
                                dep_val,
                                instance,
                                instance_path,
                                &next_schema_path,
                                depth + 1,
                                visited_refs,
                                errors,
                            );
                        }
                    }
                }
            }
        }

        // 8. Combinators: `allOf`, `anyOf`, `oneOf`, `not`
        if let Some(Value::Array(all_of_arr)) = schema_obj.get("allOf") {
            for (idx, sub_schema) in all_of_arr.iter().enumerate() {
                let next_schema_path = format!("{}/allOf/{}", schema_path, idx);
                self.validate_node(
                    sub_schema,
                    instance,
                    instance_path,
                    &next_schema_path,
                    depth + 1,
                    visited_refs,
                    errors,
                );
            }
        }

        if let Some(Value::Array(any_of_arr)) = schema_obj.get("anyOf") {
            let mut any_passed = false;
            let mut branch_errors = Vec::new();
            for (idx, sub_schema) in any_of_arr.iter().enumerate() {
                let mut sub_errs = Vec::new();
                let next_schema_path = format!("{}/anyOf/{}", schema_path, idx);
                self.validate_node(
                    sub_schema,
                    instance,
                    instance_path,
                    &next_schema_path,
                    depth + 1,
                    visited_refs,
                    &mut sub_errs,
                );
                if sub_errs.is_empty() {
                    any_passed = true;
                    break;
                } else {
                    branch_errors.push((idx, sub_errs));
                }
            }

            if !any_passed {
                let reasons = branch_errors
                    .iter()
                    .map(|(idx, errs)| format!("Option #{}: {}", idx + 1, errs[0].message))
                    .collect::<Vec<_>>()
                    .join("; ");
                errors.push(ValidationError::new(
                    instance_path,
                    format!("{}/anyOf", schema_path),
                    "anyOf",
                    format!("Instance does not match any of the allowed `anyOf` schemas (failures: {}).", reasons),
                ));
            }
        }

        if let Some(Value::Array(one_of_arr)) = schema_obj.get("oneOf") {
            let mut match_count = 0usize;
            let mut branch_errors = Vec::new();
            for (idx, sub_schema) in one_of_arr.iter().enumerate() {
                let mut sub_errs = Vec::new();
                let next_schema_path = format!("{}/oneOf/{}", schema_path, idx);
                self.validate_node(
                    sub_schema,
                    instance,
                    instance_path,
                    &next_schema_path,
                    depth + 1,
                    visited_refs,
                    &mut sub_errs,
                );
                if sub_errs.is_empty() {
                    match_count += 1;
                } else {
                    branch_errors.push((idx, sub_errs));
                }
            }

            if match_count == 0 {
                let reasons = branch_errors
                    .iter()
                    .map(|(idx, errs)| format!("Option #{}: {}", idx + 1, errs[0].message))
                    .collect::<Vec<_>>()
                    .join("; ");
                errors.push(ValidationError::new(
                    instance_path,
                    format!("{}/oneOf", schema_path),
                    "oneOf",
                    format!(
                        "Instance matched 0 `oneOf` schemas, expected exactly 1 (failures: {}).",
                        reasons
                    ),
                ));
            } else if match_count > 1 {
                errors.push(ValidationError::new(
                    instance_path,
                    format!("{}/oneOf", schema_path),
                    "oneOf",
                    format!(
                        "Instance matched {} `oneOf` schemas, expected exactly 1.",
                        match_count
                    ),
                ));
            }
        }

        if let Some(not_schema) = schema_obj.get("not") {
            let mut sub_errs = Vec::new();
            let next_schema_path = format!("{}/not", schema_path);
            self.validate_node(
                not_schema,
                instance,
                instance_path,
                &next_schema_path,
                depth + 1,
                visited_refs,
                &mut sub_errs,
            );
            if sub_errs.is_empty() {
                errors.push(ValidationError::new(
                    instance_path,
                    format!("{}/not", schema_path),
                    "not",
                    "Instance matched `not` schema (expected it NOT to match).",
                ));
            }
        }

        // 9. Conditionals: `if` / `then` / `else`
        if let Some(if_schema) = schema_obj.get("if") {
            let mut if_errs = Vec::new();
            self.validate_node(
                if_schema,
                instance,
                instance_path,
                &format!("{}/if", schema_path),
                depth + 1,
                visited_refs,
                &mut if_errs,
            );
            if if_errs.is_empty() {
                if let Some(then_schema) = schema_obj.get("then") {
                    self.validate_node(
                        then_schema,
                        instance,
                        instance_path,
                        &format!("{}/then", schema_path),
                        depth + 1,
                        visited_refs,
                        errors,
                    );
                }
            } else if let Some(else_schema) = schema_obj.get("else") {
                self.validate_node(
                    else_schema,
                    instance,
                    instance_path,
                    &format!("{}/else", schema_path),
                    depth + 1,
                    visited_refs,
                    errors,
                );
            }
        }
    }

    /// Checks if a JSON value conforms to the named schema type string.
    fn check_type(&self, expected_type: &str, instance: &Value) -> bool {
        match expected_type {
            "string" => instance.is_string(),
            "number" => instance.is_number(),
            "integer" => {
                if let Some(n) = instance.as_i64() {
                    let _ = n;
                    true
                } else if let Some(u) = instance.as_u64() {
                    let _ = u;
                    true
                } else if let Some(f) = instance.as_f64() {
                    f.fract() == 0.0
                } else {
                    false
                }
            }
            "boolean" => instance.is_boolean(),
            "null" => instance.is_null(),
            "object" => instance.is_object(),
            "array" => instance.is_array(),
            _ => true,
        }
    }

    /// Returns the human-readable type name of a JSON value.
    fn get_value_type_name(&self, val: &Value) -> &'static str {
        match val {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    "integer"
                } else {
                    "number"
                }
            }
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

// ===========================================================================
// Tool Argument Repair & Coercion Engine
// ===========================================================================

/// Options controlling argument repair and type coercion behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoercionOptions {
    /// Whether to coerce stringified numbers/booleans to their expected schema types.
    pub coerce_types: bool,
    /// Whether to populate missing fields with their `default` values defined in schema.
    pub apply_defaults: bool,
    /// Whether to remove unrecognized properties when `additionalProperties: false`.
    pub strip_unknown: bool,
    /// Whether to parse stringified JSON objects/arrays when an object/array is expected.
    pub parse_json_strings: bool,
    /// Whether to wrap single items into 1-element arrays when an array is expected.
    pub wrap_single_items_in_arrays: bool,
}

impl Default for CoercionOptions {
    fn default() -> Self {
        Self {
            coerce_types: true,
            apply_defaults: true,
            strip_unknown: false,
            parse_json_strings: true,
            wrap_single_items_in_arrays: true,
        }
    }
}

/// A recorded action taken during argument repair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoercionAction {
    pub path: String,
    pub action_type: String,
    pub description: String,
}

/// Coerces and repairs arguments against declared schema rules.
pub fn repair_and_coerce_args(
    schema: &Value,
    args: &Value,
    options: &CoercionOptions,
) -> (Value, Vec<CoercionAction>) {
    let mut actions = Vec::new();
    let repaired = repair_node(schema, args, "", options, &mut actions);
    (repaired, actions)
}

fn repair_node(
    schema: &Value,
    instance: &Value,
    path: &str,
    options: &CoercionOptions,
    actions: &mut Vec<CoercionAction>,
) -> Value {
    let schema_obj = match schema {
        Value::Object(map) => map,
        _ => return instance.clone(),
    };

    let declared_props: Option<&Map<String, Value>> =
        schema_obj.get("properties").and_then(|v| v.as_object());

    let expected_type = schema_obj.get("type").and_then(|v| v.as_str());

    let mut val = instance.clone();

    // 1. JSON string parsing for object/array
    if options.parse_json_strings && val.is_string() {
        if let Some(s) = val.as_str() {
            let trimmed = s.trim();
            if (expected_type == Some("object") || expected_type.is_none())
                && trimmed.starts_with('{')
                && trimmed.ends_with('}')
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    if parsed.is_object() {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "parse_json_string".to_string(),
                            description: "Parsed JSON string into object.".to_string(),
                        });
                        val = parsed;
                    }
                }
            } else if (expected_type == Some("array") || expected_type.is_none())
                && trimmed.starts_with('[')
                && trimmed.ends_with(']')
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    if parsed.is_array() {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "parse_json_string".to_string(),
                            description: "Parsed JSON string into array.".to_string(),
                        });
                        val = parsed;
                    }
                }
            }
        }
    }

    // 2. Primitive Type Coercion
    if options.coerce_types {
        match expected_type {
            Some("integer") => {
                if let Some(s) = val.as_str() {
                    if let Ok(int_val) = s.trim().parse::<i64>() {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "coerce_integer".to_string(),
                            description: format!(
                                "Coerced string \"{}\" to integer {}.",
                                s, int_val
                            ),
                        });
                        val = json!(int_val);
                    }
                } else if let Some(f) = val.as_f64() {
                    if f.fract() == 0.0 {
                        val = json!(f as i64);
                    }
                }
            }
            Some("number") => {
                if let Some(s) = val.as_str() {
                    if let Ok(float_val) = s.trim().parse::<f64>() {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "coerce_number".to_string(),
                            description: format!(
                                "Coerced string \"{}\" to number {}.",
                                s, float_val
                            ),
                        });
                        val = json!(float_val);
                    }
                }
            }
            Some("boolean") => {
                if let Some(s) = val.as_str() {
                    let s_lower = s.trim().to_lowercase();
                    if s_lower == "true" || s_lower == "1" || s_lower == "yes" {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "coerce_boolean".to_string(),
                            description: format!("Coerced string \"{}\" to boolean true.", s),
                        });
                        val = json!(true);
                    } else if s_lower == "false" || s_lower == "0" || s_lower == "no" {
                        actions.push(CoercionAction {
                            path: path.to_string(),
                            action_type: "coerce_boolean".to_string(),
                            description: format!("Coerced string \"{}\" to boolean false.", s),
                        });
                        val = json!(false);
                    }
                }
            }
            Some("string") => {
                if val.is_number() || val.is_boolean() {
                    let s = val.to_string();
                    actions.push(CoercionAction {
                        path: path.to_string(),
                        action_type: "coerce_string".to_string(),
                        description: format!("Coerced scalar value to string \"{}\".", s),
                    });
                    val = json!(s);
                }
            }
            Some("array") => {
                if options.wrap_single_items_in_arrays && !val.is_array() && !val.is_null() {
                    actions.push(CoercionAction {
                        path: path.to_string(),
                        action_type: "wrap_array".to_string(),
                        description: "Wrapped scalar value into 1-element array.".to_string(),
                    });
                    val = json!([val]);
                }
            }
            _ => {}
        }
    }

    // 3. Object Child Property Repair, Defaults, and Pruning
    if let Value::Object(map) = &mut val {
        let additional_allowed = schema_obj
            .get("additionalProperties")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Apply defaults for missing properties
        if options.apply_defaults {
            if let Some(props) = declared_props {
                for (prop_name, prop_schema) in props {
                    if !map.contains_key(prop_name) {
                        if let Some(def_val) = prop_schema.get("default") {
                            map.insert(prop_name.clone(), def_val.clone());
                            actions.push(CoercionAction {
                                path: format!("{}/{}", path, prop_name),
                                action_type: "apply_default".to_string(),
                                description: format!(
                                    "Applied default value for missing property `{}`: {}",
                                    prop_name, def_val
                                ),
                            });
                        }
                    }
                }
            }
        }

        // Recursively repair existing properties
        if let Some(props) = declared_props {
            for (prop_name, prop_val) in map.iter_mut() {
                if let Some(prop_schema) = props.get(prop_name) {
                    let prop_path = format!("{}/{}", path, prop_name);
                    *prop_val = repair_node(prop_schema, prop_val, &prop_path, options, actions);
                }
            }
        }

        // Strip unknown properties if requested and additionalProperties: false
        if options.strip_unknown && !additional_allowed {
            if let Some(props) = declared_props {
                let to_remove: Vec<String> = map
                    .keys()
                    .filter(|k| !props.contains_key(*k))
                    .cloned()
                    .collect();
                for k in to_remove {
                    map.remove(&k);
                    actions.push(CoercionAction {
                        path: format!("{}/{}", path, k),
                        action_type: "strip_unknown".to_string(),
                        description: format!("Stripped unknown property `{}`.", k),
                    });
                }
            }
        }
    }

    // 4. Array Item Recursion
    if let Value::Array(arr) = &mut val {
        if let Some(item_schema) = schema_obj.get("items") {
            if item_schema.is_object() {
                for (idx, item) in arr.iter_mut().enumerate() {
                    let item_path = format!("{}/{}", path, idx);
                    *item = repair_node(item_schema, item, &item_path, options, actions);
                }
            }
        }
    }
    val
}

// ===========================================================================
// Template & Scaffold Generator
// ===========================================================================

/// Generates a sample JSON template matching a given JSON Schema.
pub fn generate_schema_template(schema: &Value, include_optional: bool) -> Value {
    generate_template_node(schema, include_optional, 0)
}

fn generate_template_node(schema: &Value, include_optional: bool, depth: usize) -> Value {
    if depth > 16 {
        return json!({});
    }

    let schema_obj = match schema {
        Value::Object(map) => map,
        _ => return json!("example"),
    };

    // Check default or example first
    if let Some(def) = schema_obj.get("default") {
        return def.clone();
    }
    if let Some(ex) = schema_obj
        .get("example")
        .or_else(|| schema_obj.get("examples").and_then(|v| v.get(0)))
    {
        return ex.clone();
    }
    if let Some(Value::Array(enum_vals)) = schema_obj.get("enum") {
        if let Some(first) = enum_vals.first() {
            return first.clone();
        }
    }
    if let Some(const_val) = schema_obj.get("const") {
        return const_val.clone();
    }

    let type_name = schema_obj.get("type").and_then(|v| match v {
        Value::String(s) => Some(s.as_str()),
        Value::Array(arr) => arr.first().and_then(|v| v.as_str()),
        _ => None,
    });

    match type_name {
        Some("object") | None => {
            let mut obj = Map::new();
            let required_set: HashSet<&str> = schema_obj
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            if let Some(Value::Object(props)) = schema_obj.get("properties") {
                for (prop_name, prop_schema) in props {
                    let is_req = required_set.contains(prop_name.as_str());
                    if is_req || include_optional {
                        let sample_val =
                            generate_template_node(prop_schema, include_optional, depth + 1);
                        obj.insert(prop_name.clone(), sample_val);
                    }
                }
            }
            Value::Object(obj)
        }
        Some("array") => {
            if let Some(item_schema) = schema_obj.get("items") {
                let sample_item = generate_template_node(item_schema, include_optional, depth + 1);
                json!([sample_item])
            } else {
                json!([])
            }
        }
        Some("string") => {
            if let Some(fmt) = schema_obj.get("format").and_then(|v| v.as_str()) {
                match fmt {
                    "email" => json!("user@example.com"),
                    "uri" => json!("https://example.com/api"),
                    "ipv4" => json!("127.0.0.1"),
                    "ipv6" => json!("::1"),
                    "uuid" => json!("12345678-1234-1234-1234-123456789abc"),
                    "date-time" => json!("2026-09-02T12:00:00Z"),
                    "date" => json!("2026-09-02"),
                    "time" => json!("12:00:00"),
                    _ => json!("sample_string"),
                }
            } else if let Some(desc) = schema_obj.get("description").and_then(|v| v.as_str()) {
                json!(desc)
            } else {
                json!("sample_string")
            }
        }
        Some("integer") => json!(0),
        Some("number") => json!(0.0),
        Some("boolean") => json!(true),
        Some("null") => Value::Null,
        _ => json!("example"),
    }
}

// ===========================================================================
// Schema Summary & Markdown Generator
// ===========================================================================

/// Property metadata extracted from a JSON schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertySummary {
    pub name: String,
    pub type_desc: String,
    pub required: bool,
    pub default_val: Option<String>,
    pub description: String,
    pub constraints: Vec<String>,
}

/// Structured summary of a tool parameters schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaSummary {
    pub title: Option<String>,
    pub description: Option<String>,
    pub properties: Vec<PropertySummary>,
    pub required_fields: Vec<String>,
    pub additional_properties: bool,
}

/// Extracts a structured summary from a schema.
pub fn extract_schema_summary(schema: &Value) -> SchemaSummary {
    let schema_obj = schema.as_object();
    let title = schema_obj
        .and_then(|m| m.get("title"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let description = schema_obj
        .and_then(|m| m.get("description"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let additional_properties = schema_obj
        .and_then(|m| m.get("additionalProperties"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let required_fields: Vec<String> = schema_obj
        .and_then(|m| m.get("required"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let req_set: HashSet<&str> = required_fields.iter().map(|s| s.as_str()).collect();
    let mut properties = Vec::new();

    if let Some(props_map) = schema_obj
        .and_then(|m| m.get("properties"))
        .and_then(|v| v.as_object())
    {
        for (prop_name, prop_schema) in props_map {
            let prop_obj = prop_schema.as_object();
            let type_desc = prop_obj
                .and_then(|m| m.get("type"))
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Array(arr) => arr
                        .iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                    _ => v.to_string(),
                })
                .unwrap_or_else(|| "any".to_string());

            let prop_desc = prop_obj
                .and_then(|m| m.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let default_val = prop_obj
                .and_then(|m| m.get("default"))
                .map(|v| v.to_string());

            let mut constraints = Vec::new();
            if let Some(m) = prop_obj {
                if let Some(min) = m.get("minimum") {
                    constraints.push(format!(">= {}", min));
                }
                if let Some(max) = m.get("maximum") {
                    constraints.push(format!("<= {}", max));
                }
                if let Some(min_len) = m.get("minLength") {
                    constraints.push(format!("minLength: {}", min_len));
                }
                if let Some(max_len) = m.get("maxLength") {
                    constraints.push(format!("maxLength: {}", max_len));
                }
                if let Some(pat) = m.get("pattern").and_then(|v| v.as_str()) {
                    constraints.push(format!("pattern: `{}`", pat));
                }
                if let Some(fmt) = m.get("format").and_then(|v| v.as_str()) {
                    constraints.push(format!("format: `{}`", fmt));
                }
                if let Some(Value::Array(enums)) = m.get("enum") {
                    let enum_str = enums
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    constraints.push(format!("enum: [{}]", enum_str));
                }
            }

            properties.push(PropertySummary {
                name: prop_name.clone(),
                type_desc,
                required: req_set.contains(prop_name.as_str()),
                default_val,
                description: prop_desc,
                constraints,
            });
        }
    }

    SchemaSummary {
        title,
        description,
        properties,
        required_fields,
        additional_properties,
    }
}

/// Generates a clean Markdown reference table for a tool's parameters schema.
pub fn schema_to_markdown_docs(tool_name: &str, description: &str, parameters: &Value) -> String {
    let summary = extract_schema_summary(parameters);
    let mut md = format!("### Tool: `{}`\n\n{}\n\n", tool_name, description);

    if summary.properties.is_empty() {
        md.push_str("*(No parameters declared)*\n");
        return md;
    }

    md.push_str("| Parameter | Type | Required | Default | Description | Constraints |\n");
    md.push_str("| :--- | :--- | :---: | :--- | :--- | :--- |\n");

    for p in &summary.properties {
        let req_icon = if p.required { "**Yes**" } else { "No" };
        let def_str = p.default_val.as_deref().unwrap_or("-");
        let desc = if p.description.is_empty() {
            "-"
        } else {
            &p.description
        };
        let constraints_str = if p.constraints.is_empty() {
            "-".to_string()
        } else {
            p.constraints.join("; ")
        };

        md.push_str(&format!(
            "| `{}` | `{}` | {} | `{}` | {} | {} |\n",
            p.name, p.type_desc, req_icon, def_str, desc, constraints_str
        ));
    }

    md
}

// ===========================================================================
// Schema Diffing & Breaking Change Detection
// ===========================================================================

/// A change detected between two JSON schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaChange {
    pub path: String,
    pub change_type: String,
    pub breaking: bool,
    pub message: String,
}

/// Report summarizing differences between two schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaDiffReport {
    pub changes: Vec<SchemaChange>,
    pub has_breaking_changes: bool,
}

/// Compares two schemas and flags breaking changes.
pub fn diff_schemas(old_schema: &Value, new_schema: &Value) -> SchemaDiffReport {
    let mut changes = Vec::new();

    let old_summary = extract_schema_summary(old_schema);
    let new_summary = extract_schema_summary(new_schema);

    let old_prop_map: HashMap<&str, &PropertySummary> = old_summary
        .properties
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let new_prop_map: HashMap<&str, &PropertySummary> = new_summary
        .properties
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();

    // Check newly required properties
    for new_p in &new_summary.properties {
        if new_p.required {
            if let Some(old_p) = old_prop_map.get(new_p.name.as_str()) {
                if !old_p.required && new_p.default_val.is_none() {
                    changes.push(SchemaChange {
                        path: format!("/properties/{}", new_p.name),
                        change_type: "newly_required".to_string(),
                        breaking: true,
                        message: format!(
                            "Property `{}` was previously optional, now required without default.",
                            new_p.name
                        ),
                    });
                }
            } else if new_p.default_val.is_none() {
                changes.push(SchemaChange {
                    path: format!("/properties/{}", new_p.name),
                    change_type: "new_required_property".to_string(),
                    breaking: true,
                    message: format!(
                        "Newly added property `{}` is required without a default value.",
                        new_p.name
                    ),
                });
            }
        }
    }

    // Check removed properties
    for old_p in &old_summary.properties {
        if !new_prop_map.contains_key(old_p.name.as_str()) {
            let breaking = !new_summary.additional_properties;
            changes.push(SchemaChange {
                path: format!("/properties/{}", old_p.name),
                change_type: "removed_property".to_string(),
                breaking,
                message: format!("Property `{}` was removed from schema.", old_p.name),
            });
        }
    }

    // Check type changes
    for (name, new_p) in &new_prop_map {
        if let Some(old_p) = old_prop_map.get(name) {
            if old_p.type_desc != new_p.type_desc {
                changes.push(SchemaChange {
                    path: format!("/properties/{}/type", name),
                    change_type: "changed_type".to_string(),
                    breaking: true,
                    message: format!(
                        "Property `{}` type changed from `{}` to `{}`.",
                        name, old_p.type_desc, new_p.type_desc
                    ),
                });
            }
        }
    }

    let has_breaking_changes = changes.iter().any(|c| c.breaking);
    SchemaDiffReport {
        changes,
        has_breaking_changes,
    }
}

// ===========================================================================
// Tool Call Validation Helper Functions
// ===========================================================================

/// Validates tool call arguments against declared tool parameter schema.
pub fn validate_tool_args(
    declared_parameters: &Value,
    args: &Value,
) -> Result<(), ValidationReport> {
    let validator = JsonSchemaValidator::new(declared_parameters.clone());
    let report = validator.validate(args);
    if report.valid {
        Ok(())
    } else {
        Err(report)
    }
}

/// Validates tool call arguments against a ToolDefinition.
pub fn validate_tool_definition(
    def: &ToolDefinition,
    args: &Value,
) -> Result<(), ValidationReport> {
    validate_tool_args(&def.parameters, args)
}

/// Validates tool call arguments and attempts automatic repair/coercion if validation fails.
pub fn validate_and_repair_tool_args(
    declared_parameters: &Value,
    args: &Value,
    options: Option<CoercionOptions>,
) -> Result<(Value, ValidationReport), ValidationReport> {
    let opts = options.unwrap_or_default();
    let initial_validator = JsonSchemaValidator::new(declared_parameters.clone());
    let initial_report = initial_validator.validate(args);

    if initial_report.valid {
        return Ok((args.clone(), initial_report));
    }

    // Attempt coercion and repair
    let (repaired_args, _actions) = repair_and_coerce_args(declared_parameters, args, &opts);
    let repaired_report = initial_validator.validate(&repaired_args);

    if repaired_report.valid {
        Ok((repaired_args, repaired_report))
    } else {
        Err(repaired_report)
    }
}

/// Validates `data` against `schema`, returning `Ok(())` on success or a non-empty
/// `Vec<ValidationError>` describing every violation.
///
/// This is the ergonomic entry-point for one-shot validation without constructing a
/// [`JsonSchemaValidator`] manually.
///
/// # Example
/// ```rust,ignore
/// use serde_json::json;
/// use crate::tools::json_schema::validate;
///
/// let schema = json!({ "type": "object", "required": ["name"],
///                       "properties": { "name": { "type": "string" } } });
/// assert!(validate(&schema, &json!({ "name": "Alice" })).is_ok());
/// assert!(validate(&schema, &json!({})).is_err());
/// ```
pub fn validate(schema: &Value, data: &Value) -> Result<(), Vec<ValidationError>> {
    let validator = JsonSchemaValidator::new(schema.clone());
    let report = validator.validate(data);
    if report.valid {
        Ok(())
    } else {
        Err(report.errors)
    }
}

// ===========================================================================
// JsonSchemaTool Implementation
// ===========================================================================

/// Pure-Rust JSON Schema Validation Tool.
pub struct JsonSchemaTool {
    parameters: Value,
}

impl Default for JsonSchemaTool {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonSchemaTool {
    pub fn new() -> Self {
        Self {
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "validate",
                            "check_schema",
                            "repair",
                            "template",
                            "summary",
                            "diff"
                        ],
                        "description": "Validation or analysis action to perform.",
                        "default": "validate"
                    },
                    "schema": {
                        "description": "JSON Schema definition (object or JSON string).",
                        "type": ["object", "string"]
                    },
                    "instance": {
                        "description": "JSON instance data or tool arguments to validate / repair.",
                        "type": ["object", "array", "string", "number", "boolean", "null"]
                    },
                    "args": {
                        "description": "Alternative alias for instance data.",
                        "type": ["object", "array", "string", "number", "boolean", "null"]
                    },
                    "options": {
                        "type": "object",
                        "description": "Optional flags for repair and validation (coerce_types, apply_defaults, strip_unknown, include_optional).",
                        "properties": {
                            "coerce_types": { "type": "boolean", "default": true },
                            "apply_defaults": { "type": "boolean", "default": true },
                            "strip_unknown": { "type": "boolean", "default": false },
                            "include_optional": { "type": "boolean", "default": true }
                        }
                    },
                    "new_schema": {
                        "description": "Secondary schema for diffing breaking changes.",
                        "type": ["object", "string"]
                    }
                },
                "required": ["schema"]
            }),
        }
    }
}

#[async_trait]
impl Tool for JsonSchemaTool {
    fn name(&self) -> &str {
        "json_schema"
    }

    fn description(&self) -> &str {
        "Validates JSON values against JSON Schema specifications, checks tool call arguments, coerces malformed inputs, generates JSON templates from schemas, and audits breaking schema changes."
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("validate");

        // Parse schema
        let schema_raw = args
            .get("schema")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: `schema`"))?;
        let schema: Value = if let Value::String(s) = schema_raw {
            serde_json::from_str(s)
                .map_err(|e| anyhow::anyhow!("Invalid JSON in `schema`: {}", e))?
        } else {
            schema_raw.clone()
        };

        // Parse instance or args
        let instance_raw = args.get("instance").or_else(|| args.get("args"));
        let instance = if let Some(raw) = instance_raw {
            if let Value::String(s) = raw {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    parsed
                } else {
                    raw.clone()
                }
            } else {
                raw.clone()
            }
        } else {
            Value::Null
        };

        match action {
            "validate" => {
                let validator = JsonSchemaValidator::new(schema);
                let report = validator.validate(&instance);
                Ok(report.format_pretty())
            }
            "check_schema" => {
                // Verify schema structure
                let mut issues = Vec::new();
                if !schema.is_object() && !schema.is_boolean() {
                    issues.push("Schema must be a JSON object or boolean.".to_string());
                }
                if let Value::Object(map) = &schema {
                    if let Some(t) = map.get("type") {
                        if !t.is_string() && !t.is_array() {
                            issues.push("Schema `type` keyword must be a string or array of strings.".to_string());
                        }
                    }
                    if let Some(req) = map.get("required") {
                        if !req.is_array() {
                            issues.push("Schema `required` keyword must be an array of property names.".to_string());
                        }
                    }
                    if let Some(props) = map.get("properties") {
                        if !props.is_object() {
                            issues.push("Schema `properties` keyword must be an object.".to_string());
                        }
                    }
                }

                if issues.is_empty() {
                    Ok("✓ Schema syntax is valid JSON Schema.".to_string())
                } else {
                    Ok(format!("✗ Invalid schema syntax:\n{}", issues.iter().map(|i| format!("  - {}", i)).collect::<Vec<_>>().join("\n")))
                }
            }
            "repair" => {
                let mut opts = CoercionOptions::default();
                if let Some(opt_obj) = args.get("options").and_then(|v| v.as_object()) {
                    if let Some(c) = opt_obj.get("coerce_types").and_then(|v| v.as_bool()) {
                        opts.coerce_types = c;
                    }
                    if let Some(d) = opt_obj.get("apply_defaults").and_then(|v| v.as_bool()) {
                        opts.apply_defaults = d;
                    }
                    if let Some(s) = opt_obj.get("strip_unknown").and_then(|v| v.as_bool()) {
                        opts.strip_unknown = s;
                    }
                }

                let (repaired, actions) = repair_and_coerce_args(&schema, &instance, &opts);
                let validator = JsonSchemaValidator::new(schema);
                let report = validator.validate(&repaired);

                let mut out = format!("### Repaired JSON Instance\n```json\n{}\n```\n\n", serde_json::to_string_pretty(&repaired)?);
                out.push_str(&format!("**Validation Status**: {}\n", if report.valid { "✓ Valid" } else { "✗ Still Invalid" }));
                out.push_str(&format!("**Coercion Actions Performed**: {}\n", actions.len()));
                for (i, a) in actions.iter().enumerate() {
                    out.push_str(&format!("  {}. `{}` at `{}`: {}\n", i + 1, a.action_type, a.path, a.description));
                }

                if !report.valid {
                    out.push_str(&format!("\n**Remaining Errors**:\n{}", report.format_pretty()));
                }

                Ok(out)
            }
            "template" => {
                let include_optional = args
                    .get("options")
                    .and_then(|v| v.get("include_optional"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let template = generate_schema_template(&schema, include_optional);
                Ok(serde_json::to_string_pretty(&template)?)
            }
            "summary" => {
                let title = schema
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Schema");
                let desc = schema
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("JSON Schema parameter definition.");
                Ok(schema_to_markdown_docs(title, desc, &schema))
            }
            "diff" => {
                let new_schema_raw = args
                    .get("new_schema")
                    .ok_or_else(|| anyhow::anyhow!("`diff` action requires `new_schema` parameter."))?;
                let new_schema: Value = if let Value::String(s) = new_schema_raw {
                    serde_json::from_str(s).map_err(|e| anyhow::anyhow!("Invalid JSON in `new_schema`: {}", e))?
                } else {
                    new_schema_raw.clone()
                };

                let diff_report = diff_schemas(&schema, &new_schema);
                let mut out = format!(
                    "### Schema Diff Report\n**Breaking Changes Detected**: {}\n**Total Changes**: {}\n\n",
                    if diff_report.has_breaking_changes { "⚠️ YES" } else { "No" },
                    diff_report.changes.len()
                );

                if diff_report.changes.is_empty() {
                    out.push_str("No structural differences found between schemas.\n");
                } else {
                    for (i, c) in diff_report.changes.iter().enumerate() {
                        let icon = if c.breaking { "🚨 [BREAKING]" } else { "ℹ️ [NON-BREAKING]" };
                        out.push_str(&format!("{} {}. `{}` at `{}`: {}\n", icon, i + 1, c.change_type, c.path, c.message));
                    }
                }

                Ok(out)
            }
            _ => Err(anyhow::anyhow!("Unknown action `{}`. Supported: validate, check_schema, repair, template, summary, diff", action)),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_type_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" },
                "score": { "type": "number" },
                "is_active": { "type": "boolean" },
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["name", "age"]
        });

        let validator = JsonSchemaValidator::new(schema);

        // Valid instance
        let valid_inst = json!({
            "name": "Alice",
            "age": 30,
            "score": 98.5,
            "is_active": true,
            "tags": ["admin", "dev"]
        });
        let res = validator.validate(&valid_inst);
        assert!(res.valid, "{}", res.format_pretty());

        // Invalid types
        let invalid_inst = json!({
            "name": 123,
            "age": "thirty"
        });
        let res2 = validator.validate(&invalid_inst);
        assert!(!res2.valid);
        assert_eq!(res2.error_count, 2);
    }

    #[test]
    fn test_string_format_and_pattern() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": { "type": "string", "format": "email" },
                "uuid": { "type": "string", "format": "uuid" },
                "ipv4": { "type": "string", "format": "ipv4" },
                "code": { "type": "string", "pattern": "^[A-Z]{3}-\\d{4}$" }
            }
        });

        let validator = JsonSchemaValidator::new(schema);

        let valid_inst = json!({
            "email": "test@example.com",
            "uuid": "550e8400-e29b-41d4-a716-446655440000",
            "ipv4": "192.168.1.1",
            "code": "ABC-1234"
        });
        assert!(validator.validate(&valid_inst).valid);

        let invalid_inst = json!({
            "email": "not-an-email",
            "uuid": "invalid-uuid",
            "ipv4": "999.999.999.999",
            "code": "abc-12"
        });
        let res = validator.validate(&invalid_inst);
        assert!(!res.valid);
        assert_eq!(res.error_count, 4);
    }

    #[test]
    fn test_numeric_ranges_and_multiples() {
        let schema = json!({
            "type": "object",
            "properties": {
                "port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                "step": { "type": "number", "multipleOf": 0.5 }
            }
        });

        let validator = JsonSchemaValidator::new(schema);

        assert!(
            validator
                .validate(&json!({ "port": 8080, "step": 2.5 }))
                .valid
        );
        assert!(!validator.validate(&json!({ "port": 0 })).valid);
        assert!(!validator.validate(&json!({ "port": 70000 })).valid);
        assert!(!validator.validate(&json!({ "step": 2.3 })).valid);
    }

    #[test]
    fn test_array_items_and_unique() {
        let schema = json!({
            "type": "array",
            "items": { "type": "string" },
            "minItems": 2,
            "maxItems": 4,
            "uniqueItems": true
        });

        let validator = JsonSchemaValidator::new(schema);

        assert!(validator.validate(&json!(["a", "b", "c"])).valid);
        assert!(!validator.validate(&json!(["a"])).valid); // too short
        assert!(!validator.validate(&json!(["a", "b", "c", "d", "e"])).valid); // too long
        assert!(!validator.validate(&json!(["a", "b", "a"])).valid); // duplicate
    }

    #[test]
    fn test_schema_combinators_and_refs() {
        let schema = json!({
            "$defs": {
                "User": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" },
                        "name": { "type": "string" }
                    },
                    "required": ["id", "name"]
                }
            },
            "type": "object",
            "properties": {
                "user": { "$ref": "#/$defs/User" },
                "status": {
                    "oneOf": [
                        { "type": "string", "enum": ["pending", "active"] },
                        { "type": "integer", "minimum": 0, "maximum": 1 }
                    ]
                }
            },
            "required": ["user"]
        });

        let validator = JsonSchemaValidator::new(schema);

        let valid1 = json!({
            "user": { "id": 1, "name": "Bob" },
            "status": "active"
        });
        assert!(validator.validate(&valid1).valid);

        let valid2 = json!({
            "user": { "id": 2, "name": "Charlie" },
            "status": 1
        });
        assert!(validator.validate(&valid2).valid);

        let invalid = json!({
            "user": { "id": "not_an_int", "name": "Bob" }
        });
        assert!(!validator.validate(&invalid).valid);
    }

    #[test]
    fn test_additional_properties_and_typo_suggestions() {
        let schema = json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["file_path"],
            "additionalProperties": false
        });

        let validator = JsonSchemaValidator::new(schema);

        let with_typo = json!({
            "filepath": "/tmp/test.txt"
        });
        let res = validator.validate(&with_typo);
        assert!(!res.valid);
        let err = &res.errors[0];
        assert!(err.suggestion.as_ref().unwrap().contains("file_path"));
    }

    #[test]
    fn test_argument_repair_and_coercion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" },
                "enabled": { "type": "boolean" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "retries": { "type": "integer", "default": 3 }
            },
            "required": ["count", "enabled"]
        });

        let input_args = json!({
            "count": "42",
            "enabled": "true",
            "tags": "solo_tag"
        });

        let opts = CoercionOptions::default();
        let (repaired, actions) = repair_and_coerce_args(&schema, &input_args, &opts);

        assert_eq!(repaired["count"], 42);
        assert_eq!(repaired["enabled"], true);
        assert_eq!(repaired["tags"], json!(["solo_tag"]));
        assert_eq!(repaired["retries"], 3);
        assert!(actions.len() >= 4);

        // Validation against schema now succeeds
        let validator = JsonSchemaValidator::new(schema);
        assert!(validator.validate(&repaired).valid);
    }

    #[test]
    fn test_template_generation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "default": "My Project" },
                "count": { "type": "integer" },
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["title", "count"]
        });

        let template = generate_schema_template(&schema, true);
        assert_eq!(template["title"], "My Project");
        assert_eq!(template["count"], 0);
        assert!(template["tags"].is_array());
    }

    #[test]
    fn test_schema_diffing() {
        let old_schema = json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer" },
                "name": { "type": "string" }
            },
            "required": ["id"]
        });

        let new_schema = json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer" },
                "name": { "type": "string" },
                "email": { "type": "string" }
            },
            "required": ["id", "email"]
        });

        let report = diff_schemas(&old_schema, &new_schema);
        assert!(report.has_breaking_changes);
        assert!(report
            .changes
            .iter()
            .any(|c| c.change_type == "new_required_property"));
    }

    #[tokio::test]
    async fn test_json_schema_tool_execution() {
        let tool = JsonSchemaTool::new();
        let ctx = ToolContext::default();

        let schema = json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string" }
            },
            "required": ["cmd"]
        });

        // Test validate action
        let res = tool
            .execute(
                json!({
                    "action": "validate",
                    "schema": schema,
                    "instance": { "cmd": "cargo test" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(res.contains("✓ JSON instance is valid"));

        // Test summary action
        let summary_res = tool
            .execute(
                json!({
                    "action": "summary",
                    "schema": schema
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(summary_res.contains("| Parameter | Type | Required |"));
        assert!(summary_res.contains("`cmd`"));
    }

    // ------------------------------------------------------------------
    // Tests for the top-level `validate` free function
    // ------------------------------------------------------------------

    #[test]
    fn test_validate_fn_ok_on_valid_data() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": { "type": "string" },
                "age":  { "type": "integer", "minimum": 0 }
            },
            "additionalProperties": false
        });
        let data = json!({ "name": "Alice", "age": 30 });
        assert!(validate(&schema, &data).is_ok());
    }

    #[test]
    fn test_validate_fn_missing_required() {
        let schema = json!({
            "type": "object",
            "required": ["id"],
            "properties": { "id": { "type": "integer" } }
        });
        let errs = validate(&schema, &json!({})).unwrap_err();
        assert!(!errs.is_empty());
        assert!(errs.iter().any(|e| e.keyword == "required"));
    }

    #[test]
    fn test_validate_fn_type_mismatch() {
        let schema = json!({ "type": "string" });
        let errs = validate(&schema, &json!(42)).unwrap_err();
        assert!(!errs.is_empty());
        assert!(errs.iter().any(|e| e.keyword == "type"));
    }

    #[test]
    fn test_validate_fn_enum() {
        let schema = json!({ "type": "string", "enum": ["red", "green", "blue"] });
        assert!(validate(&schema, &json!("red")).is_ok());
        let errs = validate(&schema, &json!("yellow")).unwrap_err();
        assert!(errs.iter().any(|e| e.keyword == "enum"));
    }

    #[test]
    fn test_validate_fn_min_max() {
        let schema = json!({ "type": "number", "minimum": 1.0, "maximum": 100.0 });
        assert!(validate(&schema, &json!(50)).is_ok());
        assert!(validate(&schema, &json!(0))
            .unwrap_err()
            .iter()
            .any(|e| e.keyword == "minimum"));
        assert!(validate(&schema, &json!(101))
            .unwrap_err()
            .iter()
            .any(|e| e.keyword == "maximum"));
    }

    #[test]
    fn test_validate_fn_pattern() {
        let schema = json!({ "type": "string", "pattern": "^[A-Z]{2}\\d{3}$" });
        assert!(validate(&schema, &json!("AB123")).is_ok());
        let errs = validate(&schema, &json!("ab123")).unwrap_err();
        assert!(errs.iter().any(|e| e.keyword == "pattern"));
    }

    #[test]
    fn test_validate_fn_additional_properties_false() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "type": "integer" } },
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({ "x": 1 })).is_ok());
        let errs = validate(&schema, &json!({ "x": 1, "y": 2 })).unwrap_err();
        assert!(errs.iter().any(|e| e.keyword == "additionalProperties"));
    }

    #[test]
    fn test_validate_fn_multiple_errors_collected() {
        let schema = json!({
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": { "type": "string" },
                "b": { "type": "integer" }
            }
        });
        // Both required fields missing
        let errs = validate(&schema, &json!({})).unwrap_err();
        assert!(errs.len() >= 2, "expected >=2 errors, got {}", errs.len());
    }
}
