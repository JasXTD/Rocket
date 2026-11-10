use super::*;

fn positive(v: Option<u64>) -> Result<u64> {
    required(v.filter(|n| *n > 0 && *n <= i64::MAX as u64))
}
fn location(j: Option<&Json>, paths: &[&str]) -> Result<String> {
    let raw = j
        .and_then(|v| paths.iter().find_map(|k| v.str(k)))
        .unwrap_or("");
    if raw.len() > 8192 || raw.contains('\0') {
        return fail(ForgeFailure::InvalidResponse);
    }
    Ok(bounded(raw, 8192))
}
fn rows<'a>(j: &'a Json, key: &str) -> Result<&'a [Json]> {
    required(
        j.get(key)
            .and_then(Json::array)
            .filter(|v| v.len() <= MAX_ROWS),
    )
}
fn base(
    j: &Json,
    id: String,
    path: String,
    line: Option<u64>,
    resolved: bool,
) -> Result<ForgeReviewThread> {
    Ok(ForgeReviewThread {
        id,
        path,
        start: None,
        line,
        resolved,
        outdated: false,
        can_reply: true,
        can_resolve: !resolved,
        can_unresolve: resolved,
        comments: Vec::new(),
        limited: false,
        revision: fingerprint(j)?,
        version: None,
    })
}
fn push(
    t: &mut ForgeReviewThread,
    j: &Json,
    provider: ForgeProvider,
    parent: Option<u64>,
) -> Result<()> {
    let mut comment = required(note(j, provider))?;
    positive(Some(comment.id))?;
    if t.comments.iter().any(|c| c.id == comment.id) {
        return fail(ForgeFailure::InvalidResponse);
    }
    comment.path = (!t.path.is_empty()).then(|| t.path.clone());
    comment.line = t.line;
    comment.reply_to = parent;
    if t.comments.len() < MAX_COMMENTS {
        t.comments.push(comment);
    } else {
        t.limited = true;
    }
    Ok(())
}
pub(super) fn gitlab(j: &Json) -> Result<Option<ForgeReviewThread>> {
    let id = required(j.str("id").filter(|s| node_id(s)))?;
    let notes = rows(j, "notes")?;
    let Some(root) = notes.first() else {
        return Ok(None);
    };
    if root.bool("system") == Some(true) {
        return Ok(None);
    }
    let root_id = positive(root.u64("id"))?;
    let pos = root.get("position");
    let path = location(pos, &["new_path", "old_path"])?;
    let line = pos.and_then(|v| v.u64("new_line").or_else(|| v.u64("old_line")));
    let mut states = Vec::new();
    for n in notes.iter().filter(|n| n.bool("resolvable") == Some(true)) {
        states.push(required(n.bool("resolved"))?);
    }
    let resolvable = !states.is_empty();
    let resolved = resolvable && states.iter().all(|s| *s);
    let mut t = base(j, id.into(), path, line, resolved)?;
    t.can_resolve = resolvable && !resolved;
    t.can_unresolve = resolvable && resolved;
    for (i, note) in notes.iter().enumerate() {
        push(
            &mut t,
            note,
            ForgeProvider::GitLab,
            (i > 0).then_some(root_id),
        )?;
    }
    Ok(Some(t))
}
pub(super) fn bitbucket(items: &[Json], partial: bool) -> Result<(Vec<ForgeReviewThread>, bool)> {
    let mut ids = std::collections::HashMap::new();
    for (i, item) in items.iter().enumerate() {
        if ids.insert(positive(item.u64("id"))?, i).is_some() {
            return fail(ForgeFailure::InvalidResponse);
        }
    }
    let parent = |j: &Json| -> Result<Option<u64>> {
        match j.get("parent") {
            None | Some(Json::Null) => Ok(None),
            Some(p) => positive(p.u64("id")).map(Some),
        }
    };
    let mut grouped = std::collections::BTreeMap::<u64, Vec<&Json>>::new();
    let mut limited = partial;
    for item in items {
        let mut root = item;
        let mut visited = HashSet::new();
        while let Some(p) = parent(root)? {
            if !visited.insert(p) {
                return fail(ForgeFailure::InvalidResponse);
            }
            let Some(index) = ids.get(&p) else {
                limited = true;
                break;
            };
            root = &items[*index];
        }
        if parent(root)?.is_some() {
            continue;
        }
        grouped
            .entry(positive(root.u64("id"))?)
            .or_default()
            .push(item);
    }
    let mut out = Vec::new();
    for (id, mut comments) in grouped {
        let root = &items[ids[&id]];
        if root.bool("deleted") == Some(true) {
            continue;
        }
        comments.sort_by_key(|n| (n.u64("id") != Some(id), n.u64("id")));
        let pos = root.get("inline");
        let path = location(pos, &["path"])?;
        let line = pos.and_then(|v| v.u64("to").or_else(|| v.u64("from")));
        let resolved = match root.get("resolution") {
            None | Some(Json::Null) => false,
            Some(Json::Object(_)) => true,
            _ => return fail(ForgeFailure::InvalidResponse),
        };
        let mut t = base(
            &Json::Array(comments.iter().map(|v| (*v).clone()).collect()),
            id.to_string(),
            path,
            line,
            resolved,
        )?;
        t.start = pos.and_then(|v| v.u64("start_to").or_else(|| v.u64("start_from")));
        t.limited = limited;
        for item in comments {
            if item.bool("deleted") == Some(true) {
                continue;
            }
            push(&mut t, item, ForgeProvider::Bitbucket, parent(item)?)?;
        }
        out.push(t);
    }
    Ok((out, limited))
}
pub(super) fn server(j: &Json, anchor: Option<&Json>) -> Result<ForgeReviewThread> {
    let id = positive(j.u64("id"))?;
    let version = required(j.u64("version"))?;
    let anchor = j.get("anchor").or(anchor);
    let path = match anchor.and_then(|a| a.get("path")) {
        Some(Json::String(s)) if s.len() <= 8192 => bounded(s, 8192),
        Some(a) => {
            if let Some(p) = a.str("toString") {
                bounded(p, 8192)
            } else if let Some(parts) = a.get("components").and_then(Json::array) {
                let mut p = Vec::new();
                for part in parts {
                    if let Json::String(s) = part {
                        p.push(s.as_str());
                    } else {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                }
                let p = p.join("/");
                if p.len() > 8192 {
                    return fail(ForgeFailure::InvalidResponse);
                }
                bounded(&p, 8192)
            } else {
                return fail(ForgeFailure::InvalidResponse);
            }
        }
        _ => String::new(),
    };
    let resolved = required(j.bool("threadResolved"))?;
    let mut t = base(
        j,
        id.to_string(),
        path,
        anchor.and_then(|a| a.u64("line")),
        resolved,
    )?;
    t.version = Some(version);
    // A task has a separate state/severity workflow. Resolve only the thread;
    // never silently convert tasks or change their blocker status.
    let mut pending = vec![(j, None, 0usize)];
    let mut seen = HashSet::new();
    while let Some((v, parent, depth)) = pending.pop() {
        if depth > 16 || seen.len() >= MAX_ROWS || !seen.insert(positive(v.u64("id"))?) {
            return fail(ForgeFailure::InvalidResponse);
        }
        push(&mut t, v, ForgeProvider::BitbucketServer, parent)?;
        let children = rows(v, "comments")?;
        for child in children.iter().rev() {
            pending.push((child, Some(positive(v.u64("id"))?), depth + 1));
        }
    }
    Ok(t)
}
pub(super) fn azure(j: &Json, p: ForgeProvider) -> Result<Option<ForgeReviewThread>> {
    if j.bool("isDeleted") == Some(true) {
        return Ok(None);
    }
    let id = positive(j.u64("id"))?;
    if id > i32::MAX as u64 {
        return fail(ForgeFailure::InvalidResponse);
    }
    let notes = rows(j, "comments")?;
    let text_note = |n: &Json| {
        n.bool("isDeleted") != Some(true)
            && (n.u64("commentType") == Some(1) || n.str("commentType") == Some("text"))
    };
    let Some(root) = notes
        .iter()
        .find(|n| text_note(n) && n.u64("parentCommentId").unwrap_or(0) == 0)
    else {
        return Ok(None);
    };
    let root_id = positive(root.u64("id"))?;
    let status = match j.get("status") {
        Some(Json::String(s)) => s.as_str(),
        Some(Json::Number(n)) => match n.as_str() {
            "1" => "active",
            "2" => "fixed",
            "3" => "wontFix",
            "4" => "closed",
            "5" => "byDesign",
            "6" => "pending",
            _ => "unknown",
        },
        _ => "unknown",
    };
    let resolved = matches!(status, "fixed" | "wontFix" | "closed" | "byDesign");
    let pos = j.get("threadContext");
    let line = pos
        .and_then(|v| v.get("rightFileEnd").or_else(|| v.get("leftFileEnd")))
        .and_then(|v| v.u64("line"));
    let mut t = base(
        j,
        id.to_string(),
        location(pos, &["filePath"])?,
        line,
        resolved,
    )?;
    t.start = pos
        .and_then(|v| v.get("rightFileStart").or_else(|| v.get("leftFileStart")))
        .and_then(|v| v.u64("line"));
    // Preserve distinct server outcomes (wontFix/byDesign/closed/pending).
    // This UI offers only active <-> fixed, not arbitrary status replacement.
    t.can_resolve = status == "active";
    t.can_unresolve = status == "fixed";
    push(&mut t, root, p, None)?;
    for n in notes
        .iter()
        .filter(|n| text_note(n) && n.u64("id") != Some(root_id))
    {
        if positive(n.u64("id"))? > i16::MAX as u64 {
            return fail(ForgeFailure::InvalidResponse);
        }
        push(&mut t, n, p, n.u64("parentCommentId").filter(|n| *n > 0))?;
    }
    if root_id > i16::MAX as u64 {
        return fail(ForgeFailure::InvalidResponse);
    }
    Ok(Some(t))
}
pub(super) fn gitea(j: &Json, review: u64) -> Result<ForgeReviewThread> {
    let id = positive(j.u64("id"))?;
    if j.u64("pull_request_review_id") != Some(review) {
        return fail(ForgeFailure::InvalidResponse);
    }
    let resolved = match j.get("resolver") {
        Some(Json::Object(_)) => true,
        Some(Json::Null) => false,
        _ => return fail(ForgeFailure::InvalidResponse),
    };
    let mut t = base(
        j,
        id.to_string(),
        location(Some(j), &["path"])?,
        j.u64("position").filter(|n| *n > 0),
        resolved,
    )?;
    t.outdated = t.line.is_none();
    t.can_reply = false;
    push(&mut t, j, ForgeProvider::Gitea, None)?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json(s: &str) -> Json {
        crate::hosting::json::parse(s.as_bytes()).unwrap()
    }
    fn lab(text: &str) -> Json {
        json(&format!(
            r#"{{"id":"discussion","notes":[{{"id":8,"author":{{"username":"author"}},"body":{},"created_at":"today","resolvable":true,"resolved":false,"position":{{"new_path":"資料/新 é 🚀.rs","new_line":2}}}}]}}"#,
            quote(text)
        ))
    }
    #[test]
    fn changed_full_body_is_stale_even_when_display_is_truncated() {
        let text = "a".repeat(9000);
        let a = gitlab(&lab(&format!("{text}old"))).unwrap().unwrap();
        let b = gitlab(&lab(&format!("{text}new"))).unwrap().unwrap();
        assert_eq!(a.comments[0].body, b.comments[0].body);
        assert!(matches!(
            thread_matches(&a, &b),
            Err(Error::Forge(ForgeFailure::PullChanged))
        ));
        assert_eq!(a.path, "資料/新 é 🚀.rs");
    }
    #[test]
    fn object_field_order_does_not_invalidate_a_review() {
        assert_eq!(
            fingerprint(&json(r#"{"body":"a","id":8,"x":{"b":2,"a":1}}"#)).unwrap(),
            fingerprint(&json(r#"{"x":{"a":1,"b":2},"id":8,"body":"a"}"#)).unwrap()
        );
        assert_ne!(
            fingerprint(&json(r#"["a","bc"]"#)).unwrap(),
            fingerprint(&json(r#"["ab","c"]"#)).unwrap()
        );
        assert_ne!(
            fingerprint(&json(r#"{"value":"1"}"#)).unwrap(),
            fingerprint(&json(r#"{"value":1}"#)).unwrap()
        );
    }
    fn cloud() -> Vec<Json> {
        match json(
            r#"[{"id":8,"user":{"display_name":"author"},"content":{"raw":"original"},"parent":null,"resolution":null,"inline":{"path":"資料/界.rs","to":2}},{"id":9,"user":{"display_name":"reply author"},"content":{"raw":"reply"},"parent":{"id":8}},{"id":10,"user":{"display_name":"nested author"},"content":{"raw":"nested"},"parent":{"id":9}}]"#,
        ) {
            Json::Array(v) => v,
            _ => unreachable!(),
        }
    }
    #[test]
    fn cloud_reply_chains_keep_the_original_root_and_partial_pages_cannot_write() {
        let (a, partial) = bitbucket(&cloud(), false).unwrap();
        assert!(!partial);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].comments.len(), 3);
        assert_eq!(a[0].comments[0].id, 8);
        assert_eq!(a[0].comments[2].reply_to, Some(9));
        let (b, partial) = bitbucket(&cloud(), true).unwrap();
        assert!(partial);
        assert!(allowed(&b[0], ForgeThreadAction::Reply).is_err());
        assert!(allowed(&b[0], ForgeThreadAction::Resolve).is_err());
    }
    #[test]
    fn cloud_missing_parents_cycles_and_duplicates_never_invent_targets() {
        let mut values = cloud();
        values.remove(0);
        let (threads, limited) = bitbucket(&values, false).unwrap();
        assert!(threads.is_empty());
        assert!(limited);
        let mut values = cloud();
        values.push(values[0].clone());
        assert!(bitbucket(&values, false).is_err());
        let values = vec![
            json(r#"{"id":8,"parent":{"id":9}}"#),
            json(r#"{"id":9,"parent":{"id":8}}"#),
        ];
        assert!(bitbucket(&values, false).is_err());
    }
    #[test]
    fn gitea_resolution_preserves_stable_api_reply_limit_and_review_membership() {
        let j = json(
            r#"{"id":8,"pull_request_review_id":90,"user":{"login":"author"},"body":"text","path":"資料.rs","position":2,"resolver":null}"#,
        );
        let t = gitea(&j, 90).unwrap();
        assert!(!t.can_reply);
        assert!(t.can_resolve);
        assert!(allowed(&t, ForgeThreadAction::Reply).is_err());
        assert!(gitea(&j, 91).is_err());
    }
    #[test]
    fn azure_nonstandard_resolution_cannot_be_silently_overwritten() {
        let j = json(
            r#"{"id":7,"status":"byDesign","comments":[{"id":1,"author":{"displayName":"author"},"content":"text","commentType":"text","parentCommentId":0}],"threadContext":{"filePath":"/資料.rs","rightFileEnd":{"line":2}}}"#,
        );
        let t = azure(&j, ForgeProvider::Azure).unwrap().unwrap();
        assert!(t.resolved);
        assert!(t.can_reply);
        assert!(!t.can_resolve && !t.can_unresolve);
        assert!(allowed(&t, ForgeThreadAction::Unresolve).is_err());
    }
    #[test]
    fn server_requires_current_version_complete_children_and_known_resolution() {
        let j = json(
            r#"{"id":8,"version":3,"threadResolved":false,"author":{"displayName":"author"},"text":"text","comments":[],"anchor":{"path":{"components":["資料","新.rs"]},"line":2}}"#,
        );
        let t = server(&j, None).unwrap();
        assert_eq!(t.path, "資料/新.rs");
        assert_eq!(t.version, Some(3));
        for body in [
            r#"{"id":8,"threadResolved":false,"comments":[]}"#,
            r#"{"id":8,"version":3,"comments":[]}"#,
            r#"{"id":8,"version":3,"threadResolved":false,"author":{"name":"author"},"text":"text"}"#,
        ] {
            assert!(server(&json(body), None).is_err());
        }
    }
    #[test]
    fn thread_identifier_is_data_not_a_route() {
        let j = json(r#"{"id":"../../other/repository","notes":[]}"#);
        assert!(gitlab(&j).is_err());
    }
}
