//! The few JavaScript evaluation rules the message handlers depend on,
//! for values that came from JSON: property reads (which throw on null and
//! undefined), `for...of`, destructuring and Array.prototype.join. Errors
//! carry V8's message text, which the handlers send back as `ERROR:<msg>`.

use serde_json::Value;

use crate::util::{field, is_undefined, js_to_string};

fn null_or_undefined(p_value: Option<&Value>) -> &'static str {
    match p_value {
        Some(Value::Null) => "null",
        _ => "undefined",
    }
}

/// `v.<key>`: None is undefined; reading from null/undefined throws.
pub fn prop(p_value: Option<&Value>, p_key: &str) -> Result<Option<Value>, String> {
    match p_value {
        None | Some(Value::Null) => {
            Err(format!("Cannot read properties of {} (reading '{}')", null_or_undefined(p_value), p_key))
        }
        Some(v) if is_undefined(v) => Err(format!("Cannot read properties of undefined (reading '{}')", p_key)),
        Some(Value::String(s)) if p_key == "length" => Ok(Some(Value::from(s.encode_utf16().count()))),
        Some(Value::Array(a)) if p_key == "length" => Ok(Some(Value::from(a.len()))),
        Some(v) => Ok(field(v, p_key).cloned()),
    }
}

/// `for (const x of v)` where p_name is the source text of v.
pub fn iterate(p_value: Option<&Value>, p_name: &str) -> Result<Vec<Value>, String> {
    match p_value {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(s)) if !is_undefined(p_value.unwrap()) => {
            Ok(s.chars().map(|c| Value::String(c.to_string())).collect())
        }
        _ => Err(format!("{} is not iterable", p_name)),
    }
}

/// `const { a, b } = v` where p_name is the source text of v: throws for
/// null/undefined naming the first property.
pub fn destructure(p_value: &Value, p_name: &str, p_keys: &[&str]) -> Result<Vec<Option<Value>>, String> {
    if p_value.is_null() || is_undefined(p_value) {
        return Err(format!(
            "Cannot destructure property '{}' of '{}' as it is {}.",
            p_keys.first().copied().unwrap_or(""),
            p_name,
            if p_value.is_null() { "null" } else { "undefined" }
        ));
    }
    Ok(p_keys.iter().map(|k| field(p_value, k).cloned()).collect())
}

/// `array.join(',')`: null and undefined elements are empty.
pub fn join(p_values: &[Value]) -> String {
    p_values
        .iter()
        .map(|v| if v.is_null() || is_undefined(v) { String::new() } else { js_to_string(v) })
        .collect::<Vec<_>>()
        .join(",")
}

/// An element pushed into a JavaScript array (undefined stays undefined).
pub fn element(p_value: Option<Value>) -> Value {
    p_value.unwrap_or_else(crate::util::undefined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_match_v8() {
        assert_eq!(prop(None, "length").unwrap_err(), "Cannot read properties of undefined (reading 'length')");
        assert_eq!(
            prop(Some(&json!(null)), "length").unwrap_err(),
            "Cannot read properties of null (reading 'length')"
        );
        assert_eq!(prop(Some(&json!("ab")), "length").unwrap(), Some(json!(2)));
        assert_eq!(prop(Some(&json!(5)), "length").unwrap(), None);
        assert_eq!(prop(Some(&json!({"length": 2})), "length").unwrap(), Some(json!(2)));
        assert_eq!(iterate(Some(&json!(5)), "tasks").unwrap_err(), "tasks is not iterable");
        assert_eq!(iterate(Some(&json!({"length": 2})), "tasks").unwrap_err(), "tasks is not iterable");
        assert_eq!(iterate(Some(&json!("ab")), "tasks").unwrap(), vec![json!("a"), json!("b")]);
        assert_eq!(
            destructure(&json!(null), "task", &["taskId", "name"]).unwrap_err(),
            "Cannot destructure property 'taskId' of 'task' as it is null."
        );
        assert_eq!(destructure(&json!("x"), "task", &["taskId"]).unwrap(), vec![None]);
        assert_eq!(join(&[json!("a"), json!(null), element(None), json!(5), json!([1, 2])]), "a,,,5,1,2");
    }
}
