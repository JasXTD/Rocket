//! At most two immutable blobs per preview. Validate sizes before reading bodies;
//! use individual bodies when a combined response would exceed the caller's cap.
use super::*;

const CHECK: &str = "--batch-check=%(objectname) %(objecttype) %(objectsize)";
const BODY: &str = "--batch=%(objectname) %(objecttype) %(objectsize)";

pub(super) fn read(
    repo: &Repository,
    ids: &[ObjectId],
    cancel: &CancellationToken,
) -> Result<Vec<(ObjectId, Vec<u8>)>> {
    cancel.check()?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids.len() > 2 {
        return Err(invalid());
    }
    let header_limit: usize = ids
        .iter()
        .map(|id| id.as_str().len() + 7 + IMAGE_FILE_LIMIT.to_string().len())
        .sum();
    if repo.runner.limits.stdout_bytes < header_limit {
        // A small caller cap can fit each blob but not batch framing. Preserve
        // the original size/body reads without raising the configured cap.
        return ids
            .iter()
            .map(|id| {
                let output = repo.read_diff(
                    "image-size",
                    &args(&["--no-replace-objects", "cat-file", "-s", id.as_str()]),
                    cancel,
                )?;
                let size = std::str::from_utf8(&output)
                    .ok()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .ok_or_else(invalid)?;
                if size > IMAGE_FILE_LIMIT {
                    return Err(too_large());
                }
                body(repo, id, size, cancel)
            })
            .collect();
    }
    let input: Vec<_> = ids
        .iter()
        .flat_map(|id| id.as_str().bytes().chain(*b"\n"))
        .collect();
    let mut metadata = repo.clone();
    metadata.runner.limits.stdout_bytes = metadata.runner.limits.stdout_bytes.min(256);
    let output = metadata.read_diff_input(
        "image-size",
        &args(&["--no-replace-objects", "cat-file", CHECK]),
        input.clone(),
        cancel,
    )?;
    let sizes = sizes(&output, ids)?;
    let wire_bytes: usize = ids
        .iter()
        .zip(&sizes)
        .map(|(id, size)| id.as_str().len() + 1 + 4 + 1 + size.to_string().len() + 1 + size + 1)
        .sum();
    if wire_bytes <= repo.runner.limits.stdout_bytes {
        let mut bounded = repo.clone();
        bounded.runner.limits.stdout_bytes = wire_bytes;
        let output = bounded.read_diff_input(
            "image-blob",
            &args(&["--no-replace-objects", "cat-file", BODY]),
            input,
            cancel,
        )?;
        bodies(&output, ids, &sizes, cancel)
    } else {
        // Keep the same per-command limit as the pre-batch path, including the
        // valid boundary case of two 16 MiB blobs under the default 32 MiB cap.
        ids.iter()
            .zip(sizes)
            .map(|(id, size)| body(repo, id, size, cancel))
            .collect()
    }
}

fn body(
    repo: &Repository,
    id: &ObjectId,
    size: usize,
    cancel: &CancellationToken,
) -> Result<(ObjectId, Vec<u8>)> {
    cancel.check()?;
    let bytes = repo.read_diff(
        "image-blob",
        &args(&["--no-replace-objects", "cat-file", "blob", id.as_str()]),
        cancel,
    )?;
    if bytes.len() != size {
        return Err(invalid());
    }
    Ok((id.clone(), bytes))
}

fn header(line: &[u8], expected: &ObjectId) -> Result<usize> {
    let fields: Vec<_> = line.split(|&b| b == b' ').collect();
    if fields.len() != 3 || fields[0] != expected.as_str().as_bytes() || fields[1] != b"blob" {
        return Err(invalid());
    }
    let size = std::str::from_utf8(fields[2])
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(invalid)?;
    if size > IMAGE_FILE_LIMIT {
        return Err(too_large());
    }
    Ok(size)
}

fn sizes(mut input: &[u8], ids: &[ObjectId]) -> Result<Vec<usize>> {
    let mut sizes = Vec::with_capacity(ids.len());
    for id in ids {
        let end = input.iter().position(|&b| b == b'\n').ok_or_else(invalid)?;
        sizes.push(header(&input[..end], id)?);
        input = &input[end + 1..];
    }
    if !input.is_empty() {
        return Err(invalid());
    }
    Ok(sizes)
}

fn bodies(
    mut input: &[u8],
    ids: &[ObjectId],
    sizes: &[usize],
    cancel: &CancellationToken,
) -> Result<Vec<(ObjectId, Vec<u8>)>> {
    let mut out = Vec::with_capacity(ids.len());
    for (id, size) in ids.iter().zip(sizes) {
        cancel.check()?;
        let end = input.iter().position(|&b| b == b'\n').ok_or_else(invalid)?;
        if header(&input[..end], id)? != *size {
            return Err(invalid());
        }
        input = &input[end + 1..];
        if *size >= input.len() || input[*size] != b'\n' {
            return Err(invalid());
        }
        out.push((id.clone(), input[..*size].to_vec()));
        input = &input[*size + 1..];
    }
    if !input.is_empty() {
        return Err(invalid());
    }
    cancel.check()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_command_limits_preserve_readable_blob_pairs() {
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work/test-repositories");
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join(format!(
            "image-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let outcome = (|| -> Result<_> {
            let git = GitClient::discover()?;
            let cancel = CancellationToken::new();
            git.runner
                .run(
                    &root,
                    &args(&["init", "--template=", "--initial-branch=main"]),
                    vec![],
                    &cancel,
                )?
                .success("initialize image fixture")?;
            let mut repo = git.open(&root, &cancel)?;
            let mut ids = Vec::new();
            for bytes in [b"a\0b".as_slice(), b"c\0d"] {
                let raw = repo.read_diff_input(
                    "write fixture blob",
                    &args(&["hash-object", "-w", "--no-filters", "--stdin"]),
                    bytes.to_vec(),
                    &cancel,
                )?;
                ids.push(ObjectId::parse(std::str::from_utf8(&raw).unwrap().trim())?);
            }
            repo.runner.limits.stdout_bytes = 8;
            read(&repo, &ids, &cancel)
        })();
        // This unique directory is owned by the test, never a user repository.
        assert!(root.canonicalize().unwrap().starts_with(&base));
        fs::remove_dir_all(&root).unwrap();
        let bodies = outcome.unwrap();
        assert_eq!(bodies[0].1, b"a\0b");
        assert_eq!(bodies[1].1, b"c\0d");
    }
    #[test]
    fn batch_rejects_missing_foreign_wrong_type_length_and_trailing_data() {
        let id = ObjectId::parse(&"1".repeat(40)).unwrap();
        let ids = [id.clone()];
        let cancel = CancellationToken::new();
        for line in [
            format!("{id} missing\n"),
            format!("{} blob 1\n", "2".repeat(40)),
            format!("{id} tree 1\n"),
            format!("{id} blob {}\n", IMAGE_FILE_LIMIT + 1),
            format!("{id} blob 1\nextra"),
        ] {
            assert!(sizes(line.as_bytes(), &ids).is_err());
        }
        for body in [
            format!("{id} blob 2\nx\n"),
            format!("{id} blob 1\n"),
            format!("{id} blob 1\nx!"),
            format!("{id} blob 1\nx\nextra"),
        ] {
            assert!(bodies(body.as_bytes(), &ids, &[1], &cancel).is_err());
        }
        assert_eq!(
            bodies(
                format!("{id} blob 3\na\0b\n").as_bytes(),
                &ids,
                &[3],
                &cancel
            )
            .unwrap()[0]
                .1,
            b"a\0b"
        );
        cancel.cancel();
        assert!(matches!(
            bodies(b"", &ids, &[0], &cancel),
            Err(Error::Cancelled)
        ));
    }
}
