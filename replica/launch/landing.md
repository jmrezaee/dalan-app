# Landing page: Dalan team download page

Audience: Mehrdad's team. It is not a public marketing site and has no pricing section.
Where it lives: `site/index.html`, a static page. Serve it with GitHub Pages from the repo, or from an internal host. The download buttons point to the latest GitHub Release assets.
Voice: calm, precise, protective (`replica/brand.md`). There is no proof section: no user counts, logos or quotes, because none exist yet.

There's no `replica/fixes.md` yet (`/replica-entrepreneur` hasn't run). The feature order below is based on what this team needs, not on review research.

## 1. Hero

- **Headline:** Your servers, one keystroke away.
- **Sub:** Dalan keeps your hosts, keys, tunnels and files in one calm desktop app, encrypted on your own computer.
- **Button:** Download for macOS. The page detects the OS and shows Windows or Linux instead. Below it: "Other platforms · Release notes · v{version}".
- **Image:** a real screenshot of the app (Hosts view with a terminal open, dark theme). Until M2 ships it's a labelled placeholder. Never use a mock-up presented as the product.

## 2. The problem (our team's, in plain words)

- Server details live in five places: an old `~/.ssh/config`, a note, a chat message, someone's memory. New teammates spend their first day hunting for them.
- Keys and passwords get passed around in chat, and nobody knows which key opens which server.
- One terminal window per server, a separate app for files, and tunnels kept alive in a forgotten tab.

## 3. How it works

1. **Set a master password.** It encrypts your keys and passwords on this computer. Nothing is sent anywhere.
2. **Bring your servers in.** Import your SSH config, or add a host in a few fields. Groups pass their username, key and jump host down to every server inside.
3. **Press ⌘K and type.** Pick a server, press Enter, and you're in. Tabs, splits, files and tunnels are all one keystroke away.

## 4. Features (what's different first, the basics after)

**What's different**

- **No account, no cloud.** Your vault is one encrypted file on your machine. Dalan doesn't send telemetry.
- **Connect in one keystroke.** ⌘K, type a few letters, Enter.
- **Jump hosts and tunnels that come back.** Chains through a bastion are one setting. Tunnels reconnect by themselves after sleep or a network change.
- **Safe by default.** Every server's identity is checked. If it changes, Dalan stops and tells you why that matters.

**The basics, done well**

- Tabs, split panes and typing into many servers at once
- Files side by side, with drag and drop; edit a remote file in your own editor and Dalan uploads it when you save
- Snippets you can run on one server or twenty
- Keys: generate, import, and install on a server in one step
- Local, remote and SOCKS tunnels
- Dark and light themes, with your font and size

## 5. Pricing

Not shown. Dalan is free for the team (see `pricing.md` for what it costs to run).

## 6. FAQ

- **Where are my passwords and keys stored?** In an encrypted file on your computer, locked with your master password. They never leave it. Not even other Dalan screens can read them; only the part that makes the connection can.
- **I forgot my master password. Now what?** There's no recovery. That's the point of local encryption. You'll set up a fresh vault and re-import your servers.
- **Can I import what I already have?** Yes: your `~/.ssh/config` (with its keys and known hosts) and CSV files. Importers for other SSH clients are on the list.
- **Does it sync between my laptop and desktop?** Not yet. For now, export and import. Sync is planned, and it will be end-to-end encrypted when it comes.
- **Do I need to install anything on my servers?** No. Dalan speaks standard SSH and SFTP.
- **macOS says the app can't be verified.** Builds are signed and notarized once the Apple Developer account is set up. Until then, right-click the app, choose Open, then Open again. You only need to do this once.
- **Something broke. Where do I report it?** {internal channel / GitHub Issues link}. Include the version from Settings → About.

## 7. Final call to action

- **Headline:** Stop hunting for server details.
- **Button:** Download Dalan. Below it: "Free for the team · macOS, Windows, Linux · v{version}".

## Footer

Dalan · Built in-house · Release notes · Report a problem · Licences (Inter, JetBrains Mono, Lucide)
