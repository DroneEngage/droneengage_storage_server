//! The HTTP behaviour the dashboard inherits from Express and its
//! middleware: request bodies (body-parser), cookies (cookie-parser), the
//! query string (qs), and responses — res.send's weak ETag and 304,
//! res.redirect's content negotiation, the default error page, helmet's
//! security headers and cors.

use base64::Engine as _;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

/// helmet's Content-Security-Policy for this dashboard.
pub const CSP: &str = "default-src 'self';base-uri 'self';font-src 'self';form-action 'self';frame-ancestors 'none';img-src 'self' data:;object-src 'none';script-src 'self' 'unsafe-inline';script-src-attr 'unsafe-inline';style-src 'self' 'unsafe-inline';upgrade-insecure-requests;connect-src 'self';frame-src 'none'";

/// body-parser's default limit (100kb).
pub const BODY_LIMIT: usize = 100 * 1024;

pub struct Request {
    pub method: String,
    /// the path as sent, without the query string
    pub path: String,
    /// the raw query string (no '?')
    pub query_raw: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub ip: String,
    pub secure: bool,
}

impl Request {
    pub fn header(&self, p_name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(p_name)).map(|(_, v)| v.as_str())
    }

    /// req.query (qs)
    pub fn query(&self) -> Value {
        parse_qs(&self.query_raw)
    }

    /// req.cookies (cookie-parser): the first value of each name, decoded.
    pub fn cookies(&self) -> Vec<(String, String)> {
        let mut v_out: Vec<(String, String)> = Vec::new();
        let Some(v_header) = self.header("cookie") else { return v_out };
        for v_part in v_header.split(';') {
            let Some((k, v)) = v_part.split_once('=') else { continue };
            let k = k.trim();
            if k.is_empty() || v_out.iter().any(|(n, _)| n == k) {
                continue;
            }
            let mut v = v.trim();
            if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
                v = &v[1..v.len() - 1];
            }
            v_out.push((k.to_string(), percent_decode(v).unwrap_or_else(|| v.to_string())));
        }
        v_out
    }

    pub fn cookie(&self, p_name: &str) -> Option<String> {
        self.cookies().into_iter().find(|(k, _)| k == p_name).map(|(_, v)| v)
    }

    fn content_type(&self) -> String {
        self.header("content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase()
    }
}

/// A body parser failure (the express default error handler answers it).
pub struct HttpError {
    pub status: u16,
    pub name: &'static str,
    pub message: String,
}

/// req.body after body-parser's json() and urlencoded({extended: false}):
/// `{}` for other content types.
pub fn parse_body(p_req: &Request) -> Result<Value, HttpError> {
    let v_type = p_req.content_type();
    if p_req.body.len() > BODY_LIMIT && (v_type == "application/json" || v_type == "application/x-www-form-urlencoded")
    {
        return Err(HttpError {
            status: 413,
            name: "PayloadTooLargeError",
            message: "request entity too large".into(),
        });
    }
    match v_type.as_str() {
        "application/json" => {
            let v_text = String::from_utf8_lossy(&p_req.body);
            if v_text.trim().is_empty() {
                return Ok(Value::Object(Map::new()));
            }
            // strict: only objects and arrays
            let v_first = v_text.trim_start().chars().next().unwrap_or(' ');
            if v_first != '{' && v_first != '[' {
                return Err(HttpError {
                    status: 400,
                    name: "SyntaxError",
                    message: format!("Unexpected token '{}', \"{}\" is not valid JSON", v_first, v_text),
                });
            }
            serde_json::from_str(&v_text).map_err(|e| HttpError {
                status: 400,
                name: "SyntaxError",
                message: format!("{} (JSON)", e),
            })
        }
        "application/x-www-form-urlencoded" => Ok(parse_querystring(&String::from_utf8_lossy(&p_req.body))),
        _ => Ok(Value::Object(Map::new())),
    }
}

/// decodeURIComponent; None for malformed escapes.
pub fn percent_decode(p_text: &str) -> Option<String> {
    let b = p_text.as_bytes();
    let mut v_out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            v_out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            v_out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(v_out).ok()
}

fn decode_component(p_text: &str) -> String {
    let v = p_text.replace('+', " ");
    percent_decode(&v).unwrap_or(v)
}

fn push_value(p_obj: &mut Map<String, Value>, p_key: String, p_value: Value) {
    match p_obj.get_mut(&p_key) {
        Some(Value::Array(a)) => a.push(p_value),
        Some(v) => {
            let v_old = v.take();
            *v = Value::Array(vec![v_old, p_value]);
        }
        None => {
            p_obj.insert(p_key, p_value);
        }
    }
}

/// Node's querystring.parse: repeated keys become arrays.
pub fn parse_querystring(p_text: &str) -> Value {
    let mut m = Map::new();
    for (n, v_pair) in p_text.split('&').enumerate() {
        if n >= 1000 {
            break;
        }
        if v_pair.is_empty() {
            continue;
        }
        let (k, v) = v_pair.split_once('=').unwrap_or((v_pair, ""));
        push_value(&mut m, decode_component(k), Value::String(decode_component(v)));
    }
    Value::Object(m)
}

/// qs.parse (Express's query parser): repeated keys and `a[]` become
/// arrays, `a[b]` an object.
pub fn parse_qs(p_text: &str) -> Value {
    let mut m = Map::new();
    for v_pair in p_text.split('&').take(1000) {
        if v_pair.is_empty() {
            continue;
        }
        let (k, v) = v_pair.split_once('=').unwrap_or((v_pair, ""));
        let (k, v) = (decode_component(k), Value::String(decode_component(v)));
        match k.find('[') {
            Some(i) if i > 0 && k.ends_with(']') => {
                let (v_base, v_sub) = (k[..i].to_string(), &k[i + 1..k.len() - 1]);
                if v_sub.is_empty() {
                    match m.get_mut(&v_base) {
                        Some(Value::Array(a)) => a.push(v),
                        Some(_) => push_value(&mut m, v_base, v),
                        None => {
                            m.insert(v_base, Value::Array(vec![v]));
                        }
                    }
                } else {
                    let v_entry = m.entry(v_base).or_insert_with(|| Value::Object(Map::new()));
                    if let Value::Object(o) = v_entry {
                        push_value(o, v_sub.to_string(), v);
                    }
                }
            }
            _ => push_value(&mut m, k, v),
        }
    }
    Value::Object(m)
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// A response carrying helmet's and cors's headers (every Express response).
    pub fn new() -> Response {
        let mut r = Response { status: 200, headers: Vec::new(), body: Vec::new() };
        for (k, v) in [
            ("Content-Security-Policy", CSP),
            ("Cross-Origin-Opener-Policy", "same-origin"),
            ("Cross-Origin-Resource-Policy", "same-origin"),
            ("Origin-Agent-Cluster", "?1"),
            ("Referrer-Policy", "no-referrer"),
            ("Strict-Transport-Security", "max-age=31536000; includeSubDomains"),
            ("X-Content-Type-Options", "nosniff"),
            ("X-DNS-Prefetch-Control", "off"),
            ("X-Download-Options", "noopen"),
            ("X-Frame-Options", "SAMEORIGIN"),
            ("X-Permitted-Cross-Domain-Policies", "none"),
            ("X-XSS-Protection", "0"),
            ("Access-Control-Allow-Origin", "*"),
        ] {
            r.headers.push((k.into(), v.into()));
        }
        r
    }

    /// res.set (replaces a header, keeping its place).
    pub fn set(&mut self, p_name: &str, p_value: impl Into<String>) {
        let v = p_value.into();
        match self.headers.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(p_name)) {
            Some(h) => h.1 = v,
            None => self.headers.push((p_name.into(), v)),
        }
    }

    /// res.append (Set-Cookie).
    pub fn append(&mut self, p_name: &str, p_value: impl Into<String>) {
        self.headers.push((p_name.into(), p_value.into()));
    }

    pub fn get(&self, p_name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(p_name)).map(|(_, v)| v.as_str())
    }

    pub fn remove(&mut self, p_name: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(p_name));
    }

    /// res.send(body): Content-Type (when not set), Content-Length, weak
    /// ETag, and 304 for a fresh GET/HEAD.
    pub fn send(&mut self, p_req: &Request, p_type: &str, p_body: Vec<u8>) {
        if self.get("content-type").is_none() {
            self.set("Content-Type", p_type);
        }
        self.set("Content-Length", p_body.len().to_string());
        let v_etag = weak_etag(&p_body);
        self.set("ETag", v_etag.clone());
        self.body = p_body;
        let v_cacheable = (p_req.method == "GET" || p_req.method == "HEAD") && (200..300).contains(&self.status);
        if (v_cacheable || self.status == 304) && fresh(p_req, Some(&v_etag), None) {
            self.status = 304;
        }
        if self.status == 204 || self.status == 304 {
            self.remove("content-type");
            self.remove("content-length");
            self.body.clear();
        }
    }

    /// res.json(value)
    pub fn json(&mut self, p_req: &Request, p_value: &Value) {
        self.send(p_req, "application/json; charset=utf-8", crate::util::js_stringify(p_value).into_bytes());
    }

    /// res.send(html)
    pub fn html(&mut self, p_req: &Request, p_html: String) {
        self.send(p_req, "text/html; charset=utf-8", p_html.into_bytes());
    }

    /// res.redirect(url): 302, Location, and a body negotiated on Accept.
    pub fn redirect(&mut self, p_req: &Request, p_url: &str) {
        let v_url = encode_url(p_url);
        self.status = 302;
        self.set("Location", v_url.clone());
        self.set("Vary", "Accept");
        let v_body = match negotiate(p_req.header("accept")) {
            Some("text") => {
                self.set("Content-Type", "text/plain; charset=utf-8");
                format!("Found. Redirecting to {}", v_url)
            }
            Some(_) => {
                self.set("Content-Type", "text/html; charset=utf-8");
                format!("<p>Found. Redirecting to {}</p>", escape_html(&v_url))
            }
            None => String::new(),
        };
        self.set("Content-Length", v_body.len().to_string());
        self.body = v_body.into_bytes();
    }

    /// finalhandler's error page (without the stack trace, which the
    /// Node.js server printed in development mode).
    pub fn error_page(p_status: u16, p_name: &str, p_message: &str) -> Response {
        let mut r = Response::new();
        r.status = p_status;
        r.set("Content-Security-Policy", "default-src 'none'");
        r.set("X-Content-Type-Options", "nosniff");
        let v_body = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{}: {}</pre>\n</body>\n</html>\n",
            p_name,
            escape_html(p_message)
        );
        r.set("Content-Type", "text/html; charset=utf-8");
        r.set("Content-Length", v_body.len().to_string());
        r.body = v_body.into_bytes();
        r
    }
}

impl Default for Response {
    fn default() -> Self {
        Response::new()
    }
}

/// etag's weak tag: W/"<length hex>-<sha1 base64, 27 chars>".
pub fn weak_etag(p_body: &[u8]) -> String {
    let v_hash = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(p_body));
    format!("W/\"{:x}-{}\"", p_body.len(), &v_hash[..27])
}

/// fresh(): If-None-Match against the ETag, or If-Modified-Since.
pub fn fresh(p_req: &Request, p_etag: Option<&str>, p_last_modified: Option<i64>) -> bool {
    let v_none_match = p_req.header("if-none-match");
    let v_modified_since = p_req.header("if-modified-since");
    if v_none_match.is_none() && v_modified_since.is_none() {
        return false;
    }
    if p_req.header("cache-control").map(|c| c.contains("no-cache")).unwrap_or(false) {
        return false;
    }
    if let Some(v_nm) = v_none_match.filter(|v| *v != "*") {
        let Some(v_etag) = p_etag else { return false };
        let v_strip = |t: &str| t.trim().trim_start_matches("W/").to_string();
        if !v_nm.split(',').any(|t| v_strip(t) == v_strip(v_etag)) {
            return false;
        }
    }
    if let Some(v_ims) = v_modified_since {
        let (Some(v_lm), Some(v_since)) = (p_last_modified, parse_http_date(v_ims)) else {
            return v_none_match.is_some();
        };
        if v_lm / 1000 > v_since {
            return false;
        }
    }
    true
}

/// res.format({text, html, default}) with the request's Accept: "text",
/// "html", or None (default: empty body).
fn negotiate(p_accept: Option<&str>) -> Option<&'static str> {
    let Some(v_accept) = p_accept.filter(|a| !a.trim().is_empty()) else { return Some("text") };
    // (q, position) of the best Accept entry for a type
    let v_entries: Vec<(String, f64)> = v_accept
        .split(',')
        .map(|e| {
            let mut v_parts = e.split(';');
            let v_type = v_parts.next().unwrap_or("").trim().to_ascii_lowercase();
            let v_q = v_parts
                .filter_map(|p| p.trim().strip_prefix("q=").and_then(|q| q.parse::<f64>().ok()))
                .next()
                .unwrap_or(1.0);
            (v_type, v_q)
        })
        .collect();
    let score = |p_type: &str| -> Option<(f64, i32, usize)> {
        let (v_main, _) = p_type.split_once('/').unwrap();
        v_entries
            .iter()
            .enumerate()
            .filter_map(|(i, (t, q))| {
                let s = if t == p_type {
                    4
                } else if t == &format!("{}/*", v_main) {
                    2
                } else if t == "*/*" {
                    1
                } else {
                    return None;
                };
                Some((*q, s, i))
            })
            .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)))
            .filter(|(q, _, _)| *q > 0.0)
    };
    let v_text = score("text/plain");
    let v_html = score("text/html");
    match (v_text, v_html) {
        (None, None) => None,
        (Some(_), None) => Some("text"),
        (None, Some(_)) => Some("html"),
        (Some(t), Some(h)) => {
            if h.0 > t.0 || (h.0 == t.0 && (h.1 > t.1 || (h.1 == t.1 && h.2 < t.2))) {
                Some("html")
            } else {
                Some("text")
            }
        }
    }
}

pub fn escape_html(p_text: &str) -> String {
    p_text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// encodeurl: percent-encodes what may not appear in a URL (keeping valid
/// %XX escapes).
pub fn encode_url(p_text: &str) -> String {
    let v_bytes = p_text.as_bytes();
    let mut v_out = String::with_capacity(p_text.len());
    for (i, &b) in v_bytes.iter().enumerate() {
        let v_allowed = matches!(b, 0x21 | 0x23..=0x3B | 0x3D | 0x3F..=0x5F | 0x61..=0x7A | 0x7C | 0x7E);
        let v_escape = b == b'%'
            && i + 2 < v_bytes.len()
            && v_bytes[i + 1].is_ascii_hexdigit()
            && v_bytes[i + 2].is_ascii_hexdigit();
        if v_allowed && (b != b'%' || v_escape) {
            v_out.push(b as char);
        } else {
            v_out.push_str(&format!("%{:02X}", b));
        }
    }
    v_out
}

/// encodeURIComponent (cookie values).
pub fn encode_component(p_text: &str) -> String {
    let mut v_out = String::new();
    for b in p_text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            v_out.push(b as char);
        } else {
            v_out.push_str(&format!("%{:02X}", b));
        }
    }
    v_out
}

const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Date.prototype.toUTCString (HTTP dates).
pub fn http_date(p_ms: i64) -> String {
    let (y, mo, d, h, mi, s, _) = crate::util::civil(p_ms);
    let v_wd = DAYS[p_ms.div_euclid(86_400_000).rem_euclid(7) as usize];
    format!("{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT", v_wd, d, MONTHS[(mo - 1) as usize], y, h, mi, s)
}

/// An HTTP date in seconds since the epoch.
pub fn parse_http_date(p_text: &str) -> Option<i64> {
    let v: Vec<&str> = p_text.split_whitespace().collect();
    if v.len() < 5 {
        return None;
    }
    let d: i64 = v[1].parse().ok()?;
    let mo = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(v[2]))? as i64 + 1;
    let y: i64 = v[3].parse().ok()?;
    let t: Vec<i64> = v[4].split(':').filter_map(|x| x.parse().ok()).collect();
    if t.len() != 3 {
        return None;
    }
    // days from civil (Howard Hinnant)
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let v_days = era * 146097 + doe - 719468;
    Some(v_days * 86400 + t[0] * 3600 + t[1] * 60 + t[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(p_accept: Option<&str>) -> Request {
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_raw: String::new(),
            headers: p_accept.map(|a| vec![("Accept".to_string(), a.to_string())]).unwrap_or_default(),
            body: Vec::new(),
            ip: "127.0.0.1".into(),
            secure: true,
        }
    }

    #[test]
    fn etag_like_express() {
        assert_eq!(weak_etag(b"<div><h1>Not found.</h1></div>\n"), "W/\"1f-ChU6N+3mvo84O+wW80WDZKN2JnA\"");
    }

    #[test]
    fn redirect_negotiates_like_express() {
        let mut r = Response::new();
        r.redirect(&req(None), "/admin/login");
        assert_eq!(String::from_utf8(r.body).unwrap(), "Found. Redirecting to /admin/login");
        let mut r = Response::new();
        r.redirect(&req(Some("text/html,application/xhtml+xml,*/*;q=0.8")), "/a");
        assert_eq!(String::from_utf8(r.body).unwrap(), "<p>Found. Redirecting to /a</p>");
        let mut r = Response::new();
        r.redirect(&req(Some("application/json")), "/a");
        assert!(r.body.is_empty() && r.get("content-type").is_none());
    }

    #[test]
    fn query_strings() {
        assert_eq!(
            parse_qs("a=1&a=2&b=x+y&c[]=3&d[e]=4"),
            json!({"a": ["1", "2"], "b": "x y", "c": ["3"], "d": {"e": "4"}})
        );
        assert_eq!(parse_querystring("u=a%40b&p=&u=c"), json!({"u": ["a@b", "c"], "p": ""}));
        assert_eq!(parse_http_date(&http_date(1_793_322_309_000)), Some(1_793_322_309));
    }
}
