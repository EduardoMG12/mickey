# Mickey

One keyboard and mouse shared across several computers on the same network.
This file is the glossary: use these words in code, docs, issues and PRs.

## Language

**Peer**:
A computer running Mickey.
_Avoid_: host, client, server, machine

**Source**:
The peer whose physical keyboard and mouse are being used right now.
_Avoid_: sender, server, master

**Target**:
The peer currently receiving input from the source.
_Avoid_: receiver, client, slave

**Focus**:
Which peer is the target. Exactly one at a time; may be the source itself
(local).
_Avoid_: active screen

**Switch**:
A change of focus from one peer to another.
_Avoid_: toggle, jump, handoff

**Hotkey**:
The key combination that switches focus. Before screen edges exist it
alternates between source and target; afterwards it always brings focus back
to the source.
_Avoid_: shortcut, panic key

**Layout**:
Where each peer sits relative to the others.
_Avoid_: arrangement, topology

**Identity**:
A peer's long-term key pair. The private half never leaves the machine.
_Avoid_: token, certificate

**Fingerprint**:
A short, human-comparable hash of a peer's public key.
_Avoid_: ID, code

**Pairing**:
Confirming on both screens that the fingerprints match, after which each
peer trusts the other's key.
_Avoid_: login, registration

**Trusted peer**:
A peer whose key was paired. Only trusted peers' input is ever emitted.
_Avoid_: friend, allowed host
