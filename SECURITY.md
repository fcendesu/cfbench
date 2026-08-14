# Security policy

## Supported versions

Security fixes are provided for the latest released version of cfbench. Older
versions are not supported; users should upgrade before reporting a problem
that may already be fixed.

## Reporting a vulnerability

Do not report security vulnerabilities in a public issue, discussion, or pull
request.

Use GitHub's private **Report a vulnerability** action on the repository's
Security page when it is available. If that action is unavailable, open an
issue titled `Security contact requested` containing no vulnerability details;
the maintainer will arrange a private reporting channel. Do not send
vulnerability details through a public channel.

Include, when possible:

- The affected cfbench version and operating system.
- A clear description of the impact and affected component.
- Reproduction steps or a minimal proof of concept.
- Any known mitigations or prerequisites.
- Whether the vulnerability has been disclosed elsewhere.

You should receive an acknowledgement within seven days. The maintainer will
investigate, coordinate a fix and release where appropriate, and credit the
reporter if requested. Please allow reasonable time for remediation before any
public disclosure.

## Scope

Reports should concern cfbench's source code, release artifacts, installer,
packaging, or project-controlled automation. Vulnerabilities in Cloudflare,
GitHub, crates.io, Homebrew, operating systems, or other third-party services
should be reported to their respective maintainers.
