use super::*;
use crate::runner::Runner;
use std::{ffi::OsString, path::Path};

pub(super) fn advertise(
    runner: &Runner,
    cwd: &Path,
    destination: &[u8],
    reference: &[u8],
    cancel: &CancellationToken,
) -> Result<Option<ObjectId>> {
    Ok(advertise_many(runner, cwd, destination, &[reference.to_vec()], cancel)?.remove(0))
}
pub(super) fn advertise_many(
    runner: &Runner,
    cwd: &Path,
    destination: &[u8],
    references: &[Vec<u8>],
    cancel: &CancellationToken,
) -> Result<Vec<Option<ObjectId>>> {
    // Explicit server-option refuses silent v0 downgrade, which hides non-HEAD symrefs.
    let mut arguments = args(&[
        "-c",
        "protocol.version=2",
        "ls-remote",
        "--quiet",
        "--symref",
        "--server-option=",
        "--heads",
        "--",
    ]);
    arguments.push(bytes_to_os(destination)?);
    for reference in references {
        arguments.push(bytes_to_os(reference)?)
    }
    let output = runner.run(cwd, &arguments, vec![], cancel)?;
    if !output.status.success() && crate::tags::protocol_unsupported(&output.stderr) {
        return refused(ForcePushFailure::ProtocolRequired);
    }
    let bytes = output.success("force-push-preview")?;
    let mut groups = vec![Vec::new(); references.len()];
    for row in bytes.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        let tab = row
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| Error::parse("force-push-preview", "invalid branch advertisement"))?;
        let i = references
            .iter()
            .position(|r| r.as_slice() == &row[tab + 1..])
            .ok_or_else(|| Error::parse("force-push-preview", "unexpected branch advertisement"))?;
        groups[i].extend_from_slice(row);
        groups[i].push(b'\n');
    }
    groups
        .iter()
        .zip(references)
        .map(|(bytes, reference)| parse_advertisement(bytes, reference))
        .collect()
}

fn parse_advertisement(bytes: &[u8], reference: &[u8]) -> Result<Option<ObjectId>> {
    let invalid = || Error::parse("force-push-preview", "ambiguous branch advertisement");
    let mut id = None;
    for row in bytes.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        let separator = row.iter().position(|b| *b == b'\t').ok_or_else(invalid)?;
        let (object, name) = (&row[..separator], &row[separator + 1..]);
        if name != reference {
            return Err(invalid());
        }
        if object.starts_with(b"ref: ") {
            return refused(ForcePushFailure::SymbolicBranch);
        }
        let object = parse::oid(object, "force-push-preview")?;
        if object.as_str().bytes().all(|b| b == b'0') || id.replace(object).is_some() {
            return Err(invalid());
        }
    }
    Ok(id)
}
pub(super) fn push_arguments(
    preview: &ForcePushPreview,
    reference: &[u8],
) -> Result<Vec<OsString>> {
    let mut arguments = args(&[
        "--no-replace-objects",
        "-c",
        "push.autoSetupRemote=false",
        "-c",
        "push.negotiate=false",
        "-c",
        "push.pushOption=",
        "push",
        "--porcelain",
        "--no-force",
        "--no-force-with-lease",
        "--no-force-if-includes",
        "--no-follow-tags",
        "--no-prune",
            "--recurse-submodules=check",
        "--no-mirror",
    ]);
    arguments.push(bytes_to_os(
        &[
            b"--force-with-lease=".as_slice(),
            reference,
            b":",
            preview.expected().as_str().as_bytes(),
        ]
        .concat(),
    )?);
    arguments.push("--".into());
    arguments.push(bytes_to_os(preview.destination())?);
    arguments.push(bytes_to_os(
        &[preview.source().as_str().as_bytes(), b":", reference].concat(),
    )?);
    Ok(arguments)
}
pub(super) fn validate_porcelain(bytes: &[u8], reference: &[u8], source: &ObjectId) -> Result<()> {
    let expected = [source.as_str().as_bytes(), b":", reference].concat();
    let mut count = 0;
    for row in bytes
        .split(|b| *b == b'\n')
        .filter(|row| row.get(1) == Some(&b'\t'))
    {
        let fields: Vec<_> = row.split(|b| *b == b'\t').collect();
        if fields.len() != 3 || fields[0] != b"+" || fields[1] != expected {
            return refused(ForcePushFailure::VerificationFailed);
        }
        count += 1;
    }
    if count == 1 {
        Ok(())
    } else {
        refused(ForcePushFailure::VerificationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_advertisement_and_update_are_exact() {
        let id = ObjectId::parse(&"1".repeat(40)).unwrap();
        let reference = b"refs/heads/topic";
        let row = format!("{id}\trefs/heads/topic\n");
        assert_eq!(
            parse_advertisement(row.as_bytes(), reference).unwrap(),
            Some(id.clone())
        );
        assert_eq!(parse_advertisement(b"", reference).unwrap(), None);
        for text in [
            format!("{row}{row}"),
            format!("{id}\trefs/heads/other\n"),
            format!("{}\trefs/heads/topic\n", "0".repeat(40)),
            format!("ref: refs/heads/main\trefs/heads/topic\n{row}"),
        ] {
            assert!(parse_advertisement(text.as_bytes(), reference).is_err());
        }
        let row = format!("+\t{id}:refs/heads/topic\tOLD...NEW (forced update)\n");
        assert!(validate_porcelain(row.as_bytes(), reference, &id).is_ok());
        for bad in [
            String::new(),
            format!("{row}{row}"),
            row.replace("topic", "other"),
            row.replacen('+', "*", 1),
        ] {
            assert!(validate_porcelain(bad.as_bytes(), reference, &id).is_err());
        }
    }
}
