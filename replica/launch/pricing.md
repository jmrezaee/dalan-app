# Pricing: Dalan

**Decision: there is no price.** Dalan is free for Mehrdad's team. There are no tiers, no Stripe products and no accounts. This file instead covers what the team would otherwise pay, and what Dalan costs to run.

There is no `feedback.md` yet (`/replica-entrepreneur` hasn't run), so there's no review evidence about price and billing.

## What the alternative costs

| product | plan | price | source, read on |
| --- | --- | --- | --- |
| Termius | Starter | free (local vault, SSH, SFTP, port forwarding; no sync, no team sharing) | https://termius.com/pricing, 2026-11-04 |
| Termius | Pro | US$10/month, billed annually | same |
| Termius | Team | US$20/seat/month, billed annually, minimum 2 seats | same, and https://docs.termius.com/administration/billing-and-plans |
| Termius | Business | US$30/seat/month, billed annually | same |
| open-source clients (e.g. Tabby, Electerm) | n/a | free | their repos, not re-read for this file |

Prices change, so re-check the dates before quoting these.

Worked example, at the 2026-11-04 list price: a 5-person team on Termius Team is 5 × $20 × 12 = **US$1,200 a year**. Dalan's running cost (below) is about **US$220 a year** with signing on both Mac and Windows, or **about US$99** for Mac only. Note that Dalan has no shared team vault yet, which is the main thing the Team plan sells.

## What Dalan costs to run

| item | cost | needed for | source, read on |
| --- | --- | --- | --- |
| Apple Developer Program | US$99/year (or local equivalent) | Developer ID signing and notarization, so macOS opens the app without a warning. Also needed for Homebrew's official cask repo, which since 2026-09-01 disables casks that fail Gatekeeper | https://flaviocopes.com/apple-developer-free-vs-paid/, 2026-11-04 (blog citing Apple; confirm on developer.apple.com when enrolling) |
| Azure Artifact Signing (Windows code signing), Basic | about US$9.99/month (5,000 signatures/month) | SmartScreen-friendly signed Windows installers. Available to individuals and organisations in Canada and the USA | https://devclass.com/2026/01/14/code-signing-windows-apps-may-be-easier-and-more-secure-with-new-azure-artifact-service/, 2026-11-04. The Azure pricing page didn't render the amount, so confirm at https://azure.microsoft.com/en-ca/pricing/details/artifact-signing/ |
| Linux packages | free | AppImage, .deb, .rpm (Flathub optional) | https://v2.tauri.app/distribute/ |
| GitHub repo, Releases, Actions | free on a public repo. On a private repo, macOS runner minutes count against your plan's quota at a higher rate | CI builds and download hosting | check your GitHub plan's Actions quota before going private |
| Domain (optional) | about US$10–20/year | only if the download page needs its own address. GitHub Pages works without one | — |

**Cheapest start:**

- Skip Windows signing at first (teammates click through SmartScreen once).
- Skip Apple signing until the first build goes beyond you (right-click → Open works).
- Add Apple ($99) when the team starts using it.
- Add Azure when Windows users join.

## If this ever becomes a product

You would need these, in this order:

1. Run `/replica-entrepreneur` for the angle and the price evidence.
2. Build your own end-to-end encrypted sync, which is what people pay for in this category.
3. Then revisit this file: a free local tier, plus one paid tier for sync, priced flat and not per seat for small teams, monthly and annual.
4. Make cancelling one click from inside the app.
