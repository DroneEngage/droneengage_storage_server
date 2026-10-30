//! The dashboard's pages (views/*.html, Jinja syntax via minijinja), output
//! the way EJS printed them: `{{ x }}` escapes like EJS's `<%= %>`
//! (& < > " '), undefined and none print nothing, and files keep their
//! trailing newline.

use minijinja::value::Value as JValue;
use minijinja::{Environment, State};
use std::path::Path;

/// EJS's escapeXML.
pub fn escape(p_text: &str) -> String {
    let mut v_out = String::with_capacity(p_text.len());
    for c in p_text.chars() {
        match c {
            '&' => v_out.push_str("&amp;"),
            '<' => v_out.push_str("&lt;"),
            '>' => v_out.push_str("&gt;"),
            '"' => v_out.push_str("&#34;"),
            '\'' => v_out.push_str("&#39;"),
            c => v_out.push(c),
        }
    }
    v_out
}

fn formatter(p_out: &mut minijinja::Output, _p_state: &State, p_value: &JValue) -> Result<(), minijinja::Error> {
    if p_value.is_undefined() || p_value.is_none() {
        return Ok(());
    }
    let v_text = p_value.to_string();
    if p_value.is_safe() {
        p_out.write_str(&v_text)?;
    } else {
        p_out.write_str(&escape(&v_text))?;
    }
    Ok(())
}

/// JSON.stringify of a string, unescaped (`<%- JSON.stringify(x) %>`).
fn json_stringify(p_value: String) -> JValue {
    JValue::from_safe_string(serde_json::to_string(&p_value).unwrap_or_default())
}

pub fn environment(p_dir: &Path) -> Environment<'static> {
    let mut v_env = Environment::new();
    v_env.set_loader(minijinja::path_loader(p_dir));
    v_env.set_keep_trailing_newline(true);
    v_env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    v_env.set_formatter(formatter);
    v_env.add_filter("json_stringify", json_stringify);
    v_env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_ejs() {
        assert_eq!(escape(r#"<a href="x">'&'</a>"#), "&lt;a href=&#34;x&#34;&gt;&#39;&amp;&#39;&lt;/a&gt;");
        let v_env = environment(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/views")));
        let v_html = v_env
            .get_template("pages/404.html")
            .unwrap()
            .render(minijinja::context! { title => "404", message => "Not <found>." })
            .unwrap();
        assert_eq!(v_html, "<div><h1>Not &lt;found&gt;.</h1></div>\n");
    }
}
