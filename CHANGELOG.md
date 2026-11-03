# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1-alpha] - 2026-11-03

### Added

- Messenger style window: one conversation per contact, chat bubbles, unread counts and search.
- IMAP account setup with SMTP for sending.
- Automatic setup from DNS SRV records, the domain's autoconfig file, the Thunderbird ISP database and common server names.
- Refresh button that downloads all new mail from the Inbox and the Sent folder.
- HTML signature with a plain text version for clients that do not show HTML.
- Replies are threaded with In-Reply-To and References headers.
- Sent mail is saved to the Sent folder on the server.
- Quoted history and signatures are hidden in bubbles and can be shown on demand.
- HTML mail opens in the browser with scripts and remote images blocked.
- Local storage in SQLite.
- Password stored in the system keychain.

[Unreleased]: https://github.com/Alba-rosa/mailer/compare/v0.0.1-alpha...HEAD
[0.0.1-alpha]: https://github.com/Alba-rosa/mailer/releases/tag/v0.0.1-alpha
