# Contributing to Envelope

Envelope is a desktop mail client (Tauri v2, Rust, React) released under the **AGPL-3.0-or-later** license.
By submitting a contribution you agree that it is published under that license.

## Propose a change
1. Fork the repository and create a branch from `main`.
2. Make your changes, with tests when possible.
3. Check locally before opening the pull request:
   ```bash
   pnpm install
   pnpm build && pnpm test
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
   cargo deny check   # known vulnerabilities, licenses, sources (cargo install cargo-deny)
   ```
4. Open a pull request against `main`. Describe the problem, the solution and what you tested (including the operating system).

Only the maintainer merges. A pull request may be declined or need changes; discussions happen on the pull request.

## Project principles
- **Native first**: use what the system offers (keychain, notifications, file pickers…) or an official Tauri plugin rather than rewriting it.
- **Security**: email content is untrusted input; never store a secret in the database or in logs (system keychain only); no `unsafe` and no `unwrap()` outside tests.
- **Interface texts**: no hard-coded text. Every text goes through a key, with its **French and English** translation in the same pull request (see [TRANSLATING.md](TRANSLATING.md) to add a language). Errors raised by the Rust engine use `msg!("code", "english text")` with the code translated in `src/locales/*/errors.json`.
- **No code without a compatible license**: state where any imported code comes from.

## Report a problem
Open an issue with your system, the Envelope version, steps to reproduce and a screenshot if possible. Never post credentials or the content of real emails. Security vulnerabilities go through [SECURITY.md](SECURITY.md), not an issue.
