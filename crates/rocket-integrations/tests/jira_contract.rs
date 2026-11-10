//! Provider-shaped contracts exercised through the actual bundled Lua and VM.
//! No network/account is used; native broker/review/cancellation acceptance lives
//! in tools/tests/test-mcp-integration-workflows.cjs.
#![cfg(feature = "runtime")]

use rocket_integrations::{Node, Request, runtime};
use serde_json::{Value, json};

const SOURCE: &str = include_str!("../examples/jira/main.lua");
const CLOUD: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn context(event: &str) -> Value {
    json!({"event":event,"settings":{"project":"ROCKET"},"values":{},"state":{}})
}

fn create_context(description: &str) -> Value {
    let mut ctx = context("create");
    ctx["state"] = json!({"types":[{"id":"10001","name":"Task"}],"pane":"create"});
    ctx["values"] = json!({"issue_type":"Task","summary":"New 資料 🚀","description":description});
    ctx
}

fn drive(ctx: &Value, answer: impl Fn(&Request) -> Value) -> (Value, Vec<Request>) {
    let mut cache = serde_json::Map::new();
    let mut reads = Vec::new();
    for _ in 0..9 {
        let result = runtime::run(json!({"api":1,"source":SOURCE,"context":ctx,"responses":cache}));
        if result["kind"] != "request" {
            assert_eq!(result["kind"], "view", "{result}");
            Node::parse(&result["view"]).unwrap();
            return (result, reads);
        }
        let request = Request::parse(&result["request"]).unwrap();
        if request.writes() {
            return (result, reads);
        }
        assert!(reads.len() < 8, "exceeded the real broker's read budget");
        assert!(!cache.contains_key(&request.key()), "repeated read");
        cache.insert(request.key(), json!({"status":200,"json":answer(&request)}));
        reads.push(request);
    }
    panic!("callback did not finish within the broker budget")
}

fn offset(request: &Request) -> usize {
    request
        .path
        .split("startAt=")
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

fn search() -> Value {
    json!({"issues":[],"isLast":true})
}

fn has_text(result: &Value, text: &str) -> bool {
    fn walk(value: &Value, text: &str) -> bool {
        match value {
            Value::String(s) => s.contains(text),
            Value::Array(a) => a.iter().any(|v| walk(v, text)),
            Value::Object(o) => o.values().any(|v| walk(v, text)),
            _ => false,
        }
    }
    walk(&result["view"], text)
}

#[test]
fn server_capped_metadata_finds_required_fields_on_later_pages() {
    let (result, reads) = drive(&create_context(""), |request| {
        if request.path.contains("/search/jql?") {
            return search();
        }
        assert!(
            request
                .path
                .contains("/createmeta/ROCKET/issuetypes/10001?")
        );
        let field = match offset(request) {
            0 => json!({"fieldId":"summary","required":true}),
            1 => {
                json!({"fieldId":"customfield_10000","name":"Approval team","required":true,"hasDefaultValue":false})
            }
            _ => panic!("unexpected page"),
        };
        // Requested 50, returned one. No isLast: the published metadata examples
        // use startAt/maxResults/total and fields, unlike enhanced search.
        json!({"startAt":offset(request),"maxResults":1,"total":2,"fields":[field]})
    });
    assert_eq!(result["kind"], "view");
    assert!(has_text(&result, "needs Approval team"));
    assert_eq!(reads.len(), 3);
}

#[test]
fn required_description_blocks_empty_and_whitespace_before_review() {
    for description in ["", " \r\n\t "] {
        let (result, _) = drive(&create_context(description), |request| {
            if request.path.contains("/search/jql?") {
                return search();
            }
            json!({"startAt":0,"maxResults":50,"total":1,"fields":[
                {"fieldId":"description","name":"Description","required":true,"hasDefaultValue":false}
            ]})
        });
        assert_eq!(result["kind"], "view");
        assert!(has_text(&result, "requires a description"));
    }
}

#[test]
fn defaults_and_complete_paged_metadata_allow_the_exact_adf_create() {
    let (result, reads) = drive(&create_context("Body 資料\nSecond line"), |request| {
        let field = match offset(request) {
            0 => json!({"fieldId":"customfield_10000","required":true,"hasDefaultValue":true}),
            1 => json!({"fieldId":"description","required":true,"hasDefaultValue":false}),
            _ => panic!("unexpected page"),
        };
        json!({"startAt":offset(request),"maxResults":1,"total":2,"fields":[field]})
    });
    assert_eq!(reads.len(), 2);
    let request = Request::parse(&result["request"]).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/rest/api/3/issue");
    assert!(request.check.is_none());
    assert_eq!(
        request.body.unwrap(),
        json!({"fields":{
            "project":{"key":"ROCKET"},"issuetype":{"id":"10001"},"summary":"New 資料 🚀",
            "description":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Body 資料\nSecond line"}]}]}
        }})
    );
}

#[test]
fn incomplete_empty_or_inconsistent_metadata_never_authorizes_creation() {
    for response in [
        json!({"startAt":0,"maxResults":1,"total":2,"fields":[]}),
        json!({"startAt":1,"maxResults":1,"total":1,"fields":[]}),
        json!({"startAt":0,"maxResults":1,"total":0,"isLast":true,"fields":[{"fieldId":"summary"}]}),
        json!({"startAt":0,"maxResults":1,"fields":[{"fieldId":"summary"}]}),
        json!({"startAt":0,"maxResults":1,"total":1,"values":[{"fieldId":"summary"}]}),
    ] {
        let (result, reads) = drive(&create_context(""), |request| {
            if request.path.contains("/search/jql?") {
                search()
            } else {
                response.clone()
            }
        });
        assert_eq!(result["kind"], "view");
        assert!(has_text(&result, "could not be fully checked"));
        assert_eq!(reads.len(), 2);
    }
}

#[test]
fn metadata_budget_stops_safely_without_a_write_or_request_limit_error() {
    let (result, reads) = drive(&create_context(""), |request| {
        if request.path.contains("/search/jql?") {
            return search();
        }
        json!({"startAt":offset(request),"maxResults":1,"total":10,"fields":[{"fieldId":"summary","required":true}]})
    });
    assert_eq!(result["kind"], "view");
    assert!(has_text(&result, "could not be fully checked"));
    assert_eq!(reads.len(), 5, "four metadata pages and the normal search");
}

#[test]
fn issue_types_follow_pages_filter_subtasks_and_disclose_truncation() {
    let (result, reads) = drive(&context("create_open"), |request| {
        if request.path.contains("/search/jql?") {
            return search();
        }
        let at = offset(request);
        let types: Vec<Value> = (at..at + 20)
            .map(|i| {
                json!({
                    "id":(10000+i).to_string(),"name":format!("Type {i}"),"subtask":i==0
                })
            })
            .collect();
        json!({"startAt":at,"maxResults":20,"total":40,"issueTypes":types})
    });
    assert_eq!(reads.len(), 3);
    assert_eq!(result["state"]["types"].as_array().unwrap().len(), 32);
    assert_eq!(result["state"]["types"][0]["id"], "10001");
    assert_eq!(result["state"]["types"][31]["id"], "10032");
    assert!(has_text(&result, "Only some issue types are shown"));
}

#[test]
fn classic_and_scoped_search_keep_opaque_tokens_and_use_the_v3_gateway_path() {
    for cloud in ["", CLOUD] {
        let mut ctx = context("open");
        ctx["settings"]["cloud"] = json!(cloud.to_uppercase());
        let prefix = if cloud.is_empty() {
            "/rest/api/3".into()
        } else {
            format!("/ex/jira/{CLOUD}/rest/api/3")
        };
        let (first, reads) = drive(&ctx, |request| {
            assert!(request.path.starts_with(&format!("{prefix}/search/jql?")));
            assert!(request.path.contains("fields=summary,status"));
            json!({"issues":[],"isLast":false,"nextPageToken":"cursor/第二?x=1"})
        });
        assert_eq!(reads.len(), 1);
        ctx["event"] = json!("next_page");
        ctx["state"] = first["state"].clone();
        let (last, _) = drive(&ctx, |request| {
            assert!(
                request
                    .path
                    .contains("nextPageToken=cursor%2F%E7%AC%AC%E4%BA%8C%3Fx%3D1")
            );
            search()
        });
        assert!(last["state"]["next_token"].is_null());
    }
}

#[test]
fn invalid_cloud_id_never_requests_an_origin_or_path() {
    let mut ctx = context("open");
    ctx["settings"]["cloud"] = json!("../../another-origin");
    let (result, reads) = drive(&ctx, |_| panic!("no request allowed"));
    assert!(reads.is_empty());
    assert!(has_text(&result, "Check the Cloud ID"));
}
