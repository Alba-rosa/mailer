# Mailer

Mailer is a small email client that looks like a messenger app. Your mail is
grouped by person, and every conversation is shown as a chat with message
bubbles.

It works with any mail provider that supports IMAP and SMTP. It runs on Linux,
macOS and Windows.

## Features

- One conversation per contact, with unread counts and search.
- Setup with just your email address and password. Server settings are found
  automatically from DNS.
- A refresh button that downloads all new mail.
- HTML signatures.
- Replies stay in the same email thread.
- Quoted text and signatures are hidden in the chat, so messages stay short.
- HTML mail opens in your browser with scripts and remote images blocked.
- Your password is stored in the system keychain, not in a file.

## Download

Get the latest build from the
[Releases page](https://github.com/Alba-rosa/mailer/releases).

| System  | File                              |
| ------- | --------------------------------- |
| Linux   | `mailer-linux-x86_64.tar.gz`      |
| macOS   | `mailer-macos-arm64.tar.gz` (Apple Silicon) or `mailer-macos-x86_64.tar.gz` (Intel) |
| Windows | `mailer-windows-x86_64.zip`       |

Unpack the file and start `mailer` (or `mailer.exe` on Windows).

On macOS the app is not signed yet. If macOS blocks it, run this once in the
folder where you unpacked it:

```sh
xattr -d com.apple.quarantine mailer
```

## Setup

1. Start Mailer. The settings window opens on the first start.
2. Type your email address and click **Detect settings**. Mailer looks up the
   IMAP and SMTP servers for your domain.
3. Check the server fields. Fill them in by hand if nothing was found.
4. Type your password and click **Save**. Mailer tests the connection and then
   downloads your mail.
5. Optional: open the **Signature** tab and paste your HTML signature.

Click **Refresh** to download new mail. Click **New** to write to someone you
have not talked to yet.

### How settings are detected

Mailer tries these in order and uses the first one that works:

1. DNS SRV records (`_imaps._tcp`, `_imap._tcp`, `_submissions._tcp`,
   `_submission._tcp`).
2. The autoconfig file published by your domain.
3. The Thunderbird ISP database, also checked through your domain's MX server.
4. Common server names such as `imap.example.com` and `smtp.example.com`.

### Where data is stored

Mail and settings are stored in a SQLite database:

- Linux: `~/.local/share/mailer/mailer.db`
- macOS: `~/Library/Application Support/net.Albarosa.mailer/mailer.db`
- Windows: `%APPDATA%\Albarosa\mailer\data\mailer.db`

Set the `MAILER_DATA_DIR` environment variable to use another folder.

The password is stored in the system keychain: Keychain on macOS, Credential
Manager on Windows and Secret Service (GNOME Keyring or KWallet) on Linux. If
no keychain is available, Mailer keeps the password only until you close it.

The first refresh downloads the newest 300 messages of each folder. After that
only new messages are downloaded.

## Build from source

You need [Rust](https://rustup.rs/) 1.88 or newer.

On Debian or Ubuntu, install these packages first:

```sh
sudo apt install build-essential pkg-config libssl-dev libxkbcommon-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

Then build and run:

```sh
cargo run --release
```

Run the tests with `cargo test`.

## Releases

Versions follow [Semantic Versioning](https://semver.org/). Changes are listed
in [CHANGELOG.md](CHANGELOG.md). Pushing a tag like `v1.2.3` builds Mailer for
all systems and publishes a release with the files attached.

## License

Mailer is released under the [MIT License](LICENSE).
