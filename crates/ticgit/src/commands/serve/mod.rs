//! `ti serve` - a small read-only web view of the repo's tickets.
//!
//! Shows the same thing the TUI's issue list does (id, age, priority,
//! title, tags) plus a per-ticket detail page, served over plain HTTP
//! from a hand-rolled `std::net` listener so we pull in no web stack.
//!
//! The ticket pages live in [`tickets`] and the writeup pages in
//! [`writeups`]; this module owns the listener, the request/response
//! plumbing, and the shared page chrome both use.

mod tickets;
mod writeups;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use ticgit_lib::TicketStore;
use time::OffsetDateTime;

use crate::commands::open_store;
use crate::render::{self, NickMap};

/// How long a client gets to send its request line and headers.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on the request line + headers we're willing to read.
const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Parser)]
pub struct Args {
    /// Port to listen on. Use 0 to pick a free port.
    #[arg(short = 'p', long = "port", default_value_t = 8177)]
    pub port: u16,

    /// Address to bind. Defaults to localhost only.
    #[arg(long = "bind", default_value = "127.0.0.1")]
    pub bind: String,

    /// Open the served page in your browser.
    #[arg(long = "open")]
    pub open: bool,
}

pub fn run(args: Args) -> Result<()> {
    // Fail early (and with the usual error) if we're not in a ticgit repo.
    let store = open_store()?;
    drop(store);

    let listener = TcpListener::bind((args.bind.as_str(), args.port))
        .with_context(|| format!("binding {}:{}", args.bind, args.port))?;
    let addr = listener.local_addr()?;
    let url = format!("http://{addr}/");
    println!("ti serve: listening on {url} (ctrl-c to stop)");
    if args.open {
        open_browser(&url);
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream) {
                    eprintln!("ti serve: {err:#}");
                }
            }
            Err(err) => eprintln!("ti serve: accept failed: {err}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));

    let request = match read_request(&stream)? {
        Some(request) => request,
        None => return Ok(()),
    };

    let response = match route(&request) {
        Ok(response) => response,
        Err(err) => Response::html(500, error_page("500 - server error", &format!("{err:#}"))),
    };
    response.write_to(&mut stream)
}

/// A parsed request line: everything we care about from the client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    method: String,
    path: String,
    params: Vec<(String, String)>,
}

impl Request {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn param_values(&self, key: &str) -> Vec<String> {
        self.params
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.param(key), Some("1" | "true" | "yes" | ""))
    }
}

fn read_request(stream: &TcpStream) -> Result<Option<Request>> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    // Drain headers so the client doesn't see a reset before our response.
    let mut read = line.len();
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header)?;
        read += n;
        if n == 0 || header == "\r\n" || header == "\n" || read > MAX_HEADER_BYTES {
            break;
        }
    }
    Ok(parse_request_line(&line))
}

fn parse_request_line(line: &str) -> Option<Request> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    Some(Request {
        method,
        path: percent_decode(path),
        params: parse_query(query),
    })
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// -- routing ---------------------------------------------------------------

fn route(request: &Request) -> Result<Response> {
    if request.method != "GET" && request.method != "HEAD" {
        return Ok(Response::html(
            405,
            error_page("405 - method not allowed", "This server only answers GET."),
        ));
    }

    match request.path.as_str() {
        "/" => tickets::list_response(request),
        "/tickets.json" => tickets::json_response(request),
        "/writeups" => writeups::list_response(request),
        "/writeups.json" => writeups::json_response(request),
        "/favicon.ico" => Ok(Response::empty(204)),
        path => {
            if let Some(reference) = path.strip_prefix("/t/").filter(|r| !r.is_empty()) {
                return tickets::detail_response(reference);
            }
            if let Some(reference) = path.strip_prefix("/w/").filter(|r| !r.is_empty()) {
                return writeups::detail_response(request, reference);
            }
            Ok(Response::html(
                404,
                error_page("404 - not found", "No page at that address."),
            ))
        }
    }
}

/// Per-request context shared by both pages.
struct Page {
    repo: String,
    current_user: String,
    nicks: NickMap,
    now: OffsetDateTime,
}

impl Page {
    fn new(store: &TicketStore) -> Result<Self> {
        Ok(Self {
            repo: repo_name(),
            current_user: store.email().to_string(),
            nicks: render::build_nick_map(&store.list_users().unwrap_or_default()),
            now: OffsetDateTime::now_utc(),
        })
    }
}

fn repo_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "tickets".to_string())
}

// -- shared chrome ---------------------------------------------------------

/// A jump to the other half of the site (tickets <-> writeups), set off
/// from the view tabs it sits next to.
fn section_link(href: &str, label: &str) -> String {
    format!(
        "<a class=\"view section\" href=\"{}\">{}</a>",
        escape(href),
        escape(label)
    )
}

/// Carries the active narrowing through the search form, which would
/// otherwise drop it on submit.
fn hidden_input(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        escape(name),
        escape(value)
    )
}

/// One active filter, linking to itself removed.
fn filter_chip(label: &str, href: &str) -> String {
    format!(
        "<a class=\"chip\" href=\"{}\">{} \u{d7}</a>",
        escape(href),
        escape(label)
    )
}

/// Stable per-tag colour bucket, mirroring the TUI's tag colouring.
fn tag_hue(tag: &str) -> usize {
    tag.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as usize)
    }) % 8
}

fn error_page(title: &str, detail: &str) -> String {
    document(
        title,
        &format!(
            "<header class=\"detail\"><a class=\"back\" href=\"/\">\u{2190} all tickets</a>\
             <h1>{}</h1></header><pre class=\"prose\">{}</pre>",
            escape(title),
            escape(detail)
        ),
    )
}

fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>\n",
        escape(title)
    )
}

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--dim:#6b7280;--line:#e5e7eb;\
--accent:#2563eb;--chip:#f3f4f6;--hover:#f9fafb}\
@media(prefers-color-scheme:dark){:root{--bg:#111317;--fg:#e6e8eb;--dim:#8b93a1;--line:#262a31;\
--accent:#7aa2f7;--chip:#1c2027;--hover:#171a20}}\
*{box-sizing:border-box}\
body{margin:0;background:var(--bg);color:var(--fg);\
font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
main{max-width:1100px;margin:0 auto;padding:24px 20px 60px}\
a{color:inherit;text-decoration:none}a:hover{text-decoration:underline}\
header{display:flex;flex-wrap:wrap;gap:12px;align-items:center;\
padding-bottom:12px;border-bottom:1px solid var(--line);margin-bottom:16px}\
h1{font-size:18px;margin:0;font-weight:600}\
header nav{display:flex;gap:4px;margin-left:8px}\
nav .view{padding:3px 10px;border-radius:999px;color:var(--dim)}\
nav .view:hover{background:var(--hover);text-decoration:none}\
nav .view.active{background:var(--accent);color:#fff}\
header form{margin-left:auto}\
input[type=search]{font:inherit;padding:5px 10px;border:1px solid var(--line);\
border-radius:6px;background:var(--bg);color:var(--fg);min-width:200px}\
.filters{display:flex;gap:6px;flex-wrap:wrap;margin:-4px 0 14px}\
.chip{background:var(--chip);color:var(--dim);border-radius:999px;padding:2px 10px;font-size:12px}\
table{width:100%;border-collapse:collapse}\
th{text-align:left;font-weight:600;color:var(--dim);font-size:12px;\
text-transform:uppercase;letter-spacing:.04em;padding:6px 8px;border-bottom:1px solid var(--line)}\
th a{color:inherit}\
td{padding:6px 8px;border-bottom:1px solid var(--line);vertical-align:top}\
tbody tr:hover{background:var(--hover)}\
td.id a,td.age{color:var(--dim)}\
td.prio{color:#a855f7}td.age,td.prio,td.id{white-space:nowrap}\
td.title a{font-weight:500}\
tr.closed td.title a{color:var(--dim);text-decoration:line-through}\
td.who{color:var(--dim);white-space:nowrap}td.who.mine{color:#d97706;font-weight:600}\
.children{color:var(--dim)}\
.tag{font-size:12px;border-radius:4px;padding:1px 6px;background:var(--chip);white-space:nowrap}\
.tag-0{color:#2563eb}.tag-1{color:#0891b2}.tag-2{color:#16a34a}.tag-3{color:#ca8a04}\
.tag-4{color:#c026d3}.tag-5{color:#0ea5e9}.tag-6{color:#65a30d}.tag-7{color:#e11d48}\
.badge{font-size:12px;border-radius:4px;padding:1px 6px;background:var(--chip)}\
.state-in-progress{color:#d97706}.state-blocked{color:#dc2626}.state-review{color:#2563eb}\
.state-resolved{color:#16a34a}.state-wontfix,.state-duplicate,.state-invalid{color:var(--dim)}\
.count,.empty{color:var(--dim);margin-top:16px}\
header.detail{display:block}.back{color:var(--dim);font-size:12px}\
.subtitle{color:var(--dim);margin:6px 0 0}\
.fields{display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:10px;margin:0 0 20px}\
dt{color:var(--dim);font-size:12px;text-transform:uppercase;letter-spacing:.04em}\
dd{margin:2px 0 0}\
h2{font-size:13px;text-transform:uppercase;letter-spacing:.04em;color:var(--dim);margin:24px 0 8px}\
.prose{white-space:pre-wrap;word-wrap:break-word;font:inherit;margin:0;\
background:var(--chip);border-radius:6px;padding:12px}\
.comment{margin-bottom:12px}.byline{color:var(--dim);font-size:12px;margin:0 0 4px}\
nav .view.section{color:var(--accent)}\
.links{list-style:none;margin:0;padding:0}\
.links li{padding:5px 0;border-bottom:1px solid var(--line)}\
.links code{color:var(--dim);margin-right:8px}\
.versions{display:flex;gap:4px;flex-wrap:wrap;margin:0 0 10px}\
.vtab{background:var(--chip);color:var(--dim);border-radius:6px;padding:2px 9px;font-size:12px}\
.vtab.active{background:var(--accent);color:#fff}\
td.vers,td.who2{color:var(--dim);white-space:nowrap}\
.state-open{color:#16a34a}.state-closed{color:var(--dim)}";

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn flatten(value: &str) -> String {
    value.replace(['\n', '\r', '\t'], " ")
}

// -- responses -------------------------------------------------------------

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn new(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }

    fn html(status: u16, body: String) -> Self {
        Self::new(status, "text/html; charset=utf-8", body.into_bytes())
    }

    fn empty(status: u16) -> Self {
        Self::new(status, "text/plain; charset=utf-8", Vec::new())
    }

    fn write_to(&self, stream: &mut TcpStream) -> Result<()> {
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
             Cache-Control: no-store\r\nConnection: close\r\n\r\n",
            self.status,
            reason(self.status),
            self.content_type,
            self.body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&self.body)?;
        stream.flush()?;
        Ok(())
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn open_browser(url: &str) {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    let _ = std::process::Command::new(program)
        .args(args)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(target: &str) -> Request {
        parse_request_line(&format!("GET {target} HTTP/1.1\r\n")).unwrap()
    }

    #[test]
    fn parses_request_line_into_path_and_params() {
        let req = request("/?status=closed&tag=bug&tag=ui");
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/");
        assert_eq!(req.param("status"), Some("closed"));
        assert_eq!(req.param_values("tag"), vec!["bug", "ui"]);
    }

    #[test]
    fn percent_decoding_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn percent_encode_round_trips() {
        let value = "tag with spaces & ?=#";
        assert_eq!(percent_decode(&percent_encode(value)), value);
    }

    #[test]
    fn unknown_paths_are_404_and_non_get_is_405() {
        let response = route(&request("/nope")).unwrap();
        assert_eq!(response.status, 404);

        let post = parse_request_line("POST / HTTP/1.1\r\n").unwrap();
        assert_eq!(route(&post).unwrap().status, 405);
    }
}
