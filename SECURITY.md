# Security Policy

## Supported versions

Only the latest release receives security updates while the project is
pre-1.0.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

Email **jari@itsellesi.fi** with:

- A description of the issue
- Steps to reproduce
- Affected version (`issuectl --version`)
- Any potential impact you're aware of

You should receive an acknowledgement within 7 days. If the issue is
confirmed, we will work on a fix and coordinate a disclosure timeline
with you. Public disclosure typically happens after a fix is released.

## Threat model

`issuectl` is a local CLI that reads and writes files in a repository
under your control. It does not make network requests, run as a daemon,
or accept input from untrusted sources by default. The most relevant
risk classes are:

- **Path traversal** via crafted issue numbers, slugs, or `--root`
  values. We rely on `std::path` operations and `parse_issue_dir`
  validation; please report any case where a value can escape the
  `issues/` tree.
- **YAML parsing.** We use `serde_yaml`; report any input that causes
  panics or runaway resource use.
- **Cross-reference rewriting in `renumber`.** The tool warns about
  ambiguous duplicate numbers rather than guessing — report any case
  where it silently rewrites to the wrong target.
