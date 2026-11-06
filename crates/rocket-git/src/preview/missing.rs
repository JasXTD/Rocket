//! Fixed memory and output budgets for rev-list's object stream. The disk-size
//! override must never remove these limits, including for Keep and on-demand reads.
use crate::{Error, ObjectId, Result};

pub(super) const OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const OBJECTS: usize = 100_000;
// '?' plus a SHA-256 ID, excluding the required newline. SHA-1 is shorter.
const LINE_BYTES: usize = 65;

#[derive(Default)]
pub(super) struct MissingObjects {
    ids: Vec<ObjectId>,
    partial: Vec<u8>,
}

impl MissingObjects {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<()> {
        for part in chunk.split_inclusive(|b| *b == b'\n') {
            let complete = part.last() == Some(&b'\n');
            let part = if complete {
                &part[..part.len() - 1]
            } else {
                part
            };
            if part.len() > LINE_BYTES.saturating_sub(self.partial.len()) {
                return Err(Error::parse(
                    "preview-objects",
                    "object line exceeds its limit",
                ));
            }
            self.partial.extend_from_slice(part);
            if complete {
                let (missing, id) = match self.partial.strip_prefix(b"?") {
                    Some(id) => (true, id),
                    None => (false, self.partial.as_slice()),
                };
                if !matches!(id.len(), 40 | 64) || !id.iter().all(u8::is_ascii_hexdigit) {
                    return Err(Error::parse("preview-objects", "invalid object line"));
                }
                if missing {
                    if self.ids.len() == OBJECTS {
                        return Err(Error::Unsupported(
                            "Preview exceeds 100,000 missing objects. Use Clone to download this repository directly.".into(),
                        ));
                    }
                    self.ids.push(crate::parse::oid(id, "preview-objects")?);
                }
                self.partial.clear();
            }
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Vec<ObjectId>> {
        if !self.partial.is_empty() {
            return Err(Error::parse("preview-objects", "incomplete object line"));
        }
        Ok(self.ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_preserve_sha1_and_sha256_missing_objects_and_ignore_present_objects() {
        let a = "a".repeat(40);
        let b = "b".repeat(64);
        let data = format!("{a}\n?{a}\n{b}\n?{b}\n");
        for size in [1, 2, 41, 64, 8192] {
            let mut list = MissingObjects::default();
            for chunk in data.as_bytes().chunks(size) {
                list.push(chunk).unwrap();
            }
            let ids: Vec<_> = list
                .finish()
                .unwrap()
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect();
            assert_eq!(ids, [a.clone(), b.clone()]);
        }
    }

    #[test]
    fn exact_object_budget_succeeds_and_the_next_object_is_refused() {
        let mut list = MissingObjects::default();
        for n in 0..OBJECTS {
            list.push(format!("?{n:064x}\n").as_bytes()).unwrap();
        }
        assert_eq!(list.ids.len(), OBJECTS);
        // Present objects don't consume the missing-object budget.
        list.push(format!("{:064x}\n", OBJECTS).as_bytes()).unwrap();
        let ids = list.finish().unwrap();
        assert_eq!(ids.len(), OBJECTS);
        let mut list = MissingObjects {
            ids,
            partial: Vec::new(),
        };
        assert!(matches!(
            list.push(format!("?{:064x}\n", OBJECTS).as_bytes()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(list.ids.len(), OBJECTS, "no extra object was appended");
    }

    #[test]
    fn malformed_unterminated_and_overlong_lines_fail_closed() {
        for invalid in [
            b"?bad\n".as_slice(),
            b"\n",
            b"?gggggggggggggggggggggggggggggggggggggggg\n",
        ] {
            assert!(MissingObjects::default().push(invalid).is_err());
        }
        let mut list = MissingObjects::default();
        list.push(format!("?{}", "a".repeat(64)).as_bytes())
            .unwrap();
        assert!(list.push(b"b").is_err());
        assert!(list.partial.len() <= LINE_BYTES);
        let mut list = MissingObjects::default();
        assert!(list.push(&[b'a'; 8192]).is_err());
        assert!(list.partial.is_empty());
        let mut list = MissingObjects::default();
        list.push(format!("?{}", "a".repeat(40)).as_bytes())
            .unwrap();
        assert!(list.finish().is_err());
    }
}
