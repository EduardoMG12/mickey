# The release key

Mickey installs an update only when the release's `SHA256SUMS` carries a
valid Ed25519 signature from the key in
[`packaging/release-key.pub`](../../packaging/release-key.pub) (PRD 0013).

| Script | Does |
|---|---|
| `new-key.sh` | Makes the key once: the private half to `~/.config/mickey-release/` and the `RELEASE_SIGNING_KEY` secret, the public half to `packaging/release-key.pub` |
| `sign.sh` | Signs a `SHA256SUMS` (release.yml runs it), then checks the signature |
| `verify.sh` | Checks a downloaded release's signature, as Mickey does |

## What the signature protects against

- **A changed download** (a swapped file on GitHub, a proxy): refused, the
  hash no longer matches the signed list.
- **An older release served as a newer one**: refused, the signed list names
  its own version (`Mickey-Setup-<version>.exe`).

## What it does not, on its own

The private key is a GitHub secret, and the release workflow signs what it
built. Someone who takes over the maintainer's GitHub account or a token
that can push tags or workflows could have a release built and signed.
Two settings close most of that; both are done once, on GitHub, by the
maintainer:

1. **Make the key need your approval.**
   - Settings → Environments → **New environment**, named `release`.
   - Turn on **Required reviewers** and add yourself.
   - Under **Deployment branches and tags**, choose **Selected branches and
     tags** and add the tag rule `v*`.
   - Under **Environment secrets**, add `RELEASE_SIGNING_KEY` with the
     contents of `~/.config/mickey-release/release-key.pem`.
   - Delete the repository secret of the same name
     (Settings → Secrets and variables → Actions).

   The `publish` job then waits for your click before it can read the key,
   so a stolen token alone cannot sign.
2. **Keep two-factor authentication on** the GitHub account.

The actions the workflows use are pinned by commit, so a moved tag upstream
cannot change what runs next to the key.

For the strongest setup, sign on your own PC instead: let the workflow
publish a draft release without signing, download `SHA256SUMS`, run
`sign.sh ~/.config/mickey-release/release-key.pem SHA256SUMS`, upload
`SHA256SUMS.sig` and publish. The key then never leaves your PC.

## If the key is lost or leaks

- **Lost:** make a new one with `new-key.sh` (move the old `.pem` away
  first) and release. Installed Mickeys know only the old key, so each PC
  installs that release by hand once.
- **Leaked:** the same, as soon as possible, and say so in the release
  notes. A Mickey with the old key would accept a release signed with the
  leaked one until it is updated.
