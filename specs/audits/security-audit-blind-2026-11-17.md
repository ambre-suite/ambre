# AMBRE -- SECURITY AUDIT: THE BLIND WORKSPACE PASS OF 2026-11-17

> An addendum to [`security-audit.md`](security-audit.md), which holds the numbered findings and
> the earlier passes. Each `N` and `P` identifier below 26 is declared there.

This addendum starts the A security pre-audit of the whole workspace Story (`ambre-roadmap.md`
MVP). It is an audit alone: this pass fixes nothing, and ticks no box. The Story stays open, and a
delta pass follows the last batch of the milestone.

**Method.** Five AI reviewers worked apart, each on one scope, read-only. None read
`security-audit.md`, the roadmap, the journal or the commit history before it wrote its list. Each
list was closed before the comparison below. So the overlap with the earlier passes measures what those passes
left unseen.

**Scopes.** The relay and the zero-knowledge boundary. The key hierarchy, the rotations and the
sealed envelopes. The calls, the SFU and the call worker. The parsers of foreign files. The
client: the threads, the bus, the local store and the remote deltas of each application.

**Threats, in this order.** A malicious relay, which replays, reorders, swaps and forges what it
carries. A removed member, alone or with the relay. A hostile file, or hostile content from a
current member.

**Evidence.** No finding below was run. Each comes from a read of the code on 2026-11-17. The
author of this addendum read the code again for `P26` to `P32`, and confirmed each path. The other
findings stand on the read of one reviewer.

**The cast.** On 2026-11-17, `ambre-spec-zero-knowledge.md section 2` cast the relay as honest but
curious. This pass casts a malicious relay, as the three passes before it did. Since 2026-11-18,
that table casts the relay as malicious too.

## Findings

Every finding was open on 2026-11-17, so each takes a `P` identifier. A path is relative to
`crates/`. `Class` compares the blind list with the findings of `security-audit.md`. A finding is of
the first milestone only when it is high and it breaks the zero-knowledge boundary or lets one
member act as another. `State` reads `open`, or names the identifier a fixed finding took in
`security-audit.md`.

| Ref | Severity | Milestone | Class | Where | Scenario | State |
| :-- | :-- | :-- | :-- | :-- | :-- | :-- |
| P26 | high | MVP | new, the shape of `N15` and `N27` | `ambre-sync/src/sharing.rs:112` (`served_log`), `:1394` (`settle_task`), `:819` (`revoke_task`), `:552` (`invite_task`), `:245` (`rotate_entries_for`) | The relay answers that a rotation is owed. The settle reads the members again and roots the log on the anchor of that fresh answer. The relay serves a wrap of its own, with the anchor of a log it signed. That log signs exchange keys of the relay. The client wraps the fresh key for those keys, and no check of an open runs. The link of the same request seals the previous key under the fresh one. So the relay alone reads the document from then on, and its history through the key chain. A revoke, an invite and a role change take the same path. | fixed as `N32` |
| P27 | high | MVP | known in part: the limit of `N23` and `P24` | `ambre-sync/src/sharing.rs:1394` (`settle_task`), `:819` (`revoke_task`); `ambre-sync/src/withheld.rs:162` (`HeadWatch::served`) | Eve was revoked at one place of the log. Later the relay names a debt to a writer. The relay serves the genuine log cut before that place, with the row of Eve. The cut log is valid from the true anchor and signs the exchange key of Eve. The rotation wraps the fresh key for Eve, and the link gives her the key in force. No path compares the served log with the head this client already walked, or signed itself. | fixed as `N33` |
| P28 | high | MVP | new | `ambre-crypto/src/hash.rs:152` (`dev_seed`); `ambre-sync/src/auth.rs:113` (`handle_dev_login`); `ambre-app/src/lib.rs:880`; `ambre-app/src/platform.rs:207`, `:333` | No `cfg` and no feature removes the development sign-in from a release build. A link with `?user=name`, or a stored name, sends `DevLogin` at boot. The master seed is a hash of that name, so anyone who knows the name derives every key. The relay is the one gate: a malicious relay accepts the sign-in, or answers that development mode is on. The session then signs, wraps and seals under keys the relay derives. The name is stored before the answer, so each later boot signs in the same way. | fixed as `N34` |
| P29 | high | MVP | known in part: `P24` holds the roll back, the freeze is new | `ambre-sync/src/call_link.rs:436` (`call_log`), `:304`; `ambre-sync/src/websocket.rs:1784`, `:1823`; `ambre-sync/src/sharing.rs:1660` (`rooted_call_log`) | A call reads the log of its channel at the join, then only on two frames the relay pushes. An owner removes Eve during the call, and the relay pushes neither frame. No client reads the log again, the client that signed the removal included. So no participant draws a fresh sender key, and Eve opens the call to its end. A later read also replaces the held log with no comparison, so an older genuine log opens the door to a new join of Eve. | fixed as `N35` |
| P30 | high | MVP | known: `P17`, rated low there | `ambre-sfu/src/forward.rs:1312` (`names_another_sender`), `:1341` (`vp8_frame_start`); `ambre-sfu/src/lib.rs:27` (`participant_config`) | Mallory holds the sender key of Bob, as every participant does. She seals a video frame under the identifier of Bob. She cuts the first packet of the frame before the end of the chunk header, or negotiates a codec other than VP8. The filter reads no header, and a packet with no header passes. The receiver joins the packets again and opens the frame under the key of Bob. An honest SFU forwards it. A high counter also moves the replay window, so the true video of Bob drops. | fixed as `N37` |
| P31 | high | MVP | regression: `P7` reads fixed, and the fix waits on the attacker | `ambre-crypto/src/call.rs:641` (`open_as`), `:532` (`ReceiveKeys::insert`); `ambre-call/src/lib.rs:123` (`KeyHandoff::Open`) | Eve leaves a call, and Alice draws a key of a fresh epoch. A receiver drops the older epoch only once a chunk opens under the fresh one. A malicious SFU forwards no chunk of the fresh epoch to Roger. The SFU sends him chunks that Eve seals under the older key of Alice. Each one opens and reads as a chunk of Alice, for the rest of the call. | fixed as `N36` |
| P32 | high | MVP, for the owner to rule | regression: `N7` reads fixed, and one state still passes | `ambre-server/src/routes/documents.rs:301` (`create_document`); `ambre-server/src/routes/share.rs:41` (`checked_entry`); `ambre-server/src/db/channels.rs:1315`; `ambre-server/src/db/members.rs:589` | Mallory registers a public channel, so she owns the row of its `meta`. The `meta` of a public channel may hold no member row, as `SRV-173` allows. Mallory leaves it as its last member, and an owner removes her from the space. She posts the creation of the `meta` again, with no entry. The route finds no member row and registers her as owner. She then reads each document of the channel, writes to it and deletes the channel. A public channel owes no rotation, so she still holds its key. | fixed as `N38` |
| P33 | high | V1 | new | `ambre-trame/src/formula/functions/builtins_b.rs:75` (`rept`); `ambre-trame/src/formula/vm.rs:279` (`Op::Concat`) | An editor writes `=REPT("x",3000000000)`, or a chain of forty cells that each double a text. The delta is signed and valid. Each peer evaluates the formula on its Logic thread and allocates with no bound. The thread traps, and the document opens for nobody. | open |
| P34 | high | V1 | new | `ambre-trame/src/formula/vm.rs:143` (`eval_cell`), `:331` (`eval_range`) | An editor writes a chain of cells, each one a `SUM` over a range of one cell. Range edges sit outside the cell graph, so the order of the pass does not see them. The evaluation recurses once per cell of the chain. A chain of some ten thousand cells overflows the stack of the Logic thread. | open |
| P35 | high | V1 | new | `ambre-trame/src/logic/merges.rs:208` (`resolved_merges`), `:167`, `:264` | An editor writes one merge entry over 30 000 rows and 20 000 columns. Each peer lists every cell of that area into two sets at open. The Logic thread runs out of memory. | open |
| P36 | high | V1 | new | `ambre-prose/src/layout/pass.rs:659` (`walk`); `ambre-prose/src/logic/tables.rs:146`, `:155` | An editor writes some ten thousand blocks, each one the child of the block before. The layout walks the block tree by recursion, with no depth bound. The stack of the Logic thread overflows at each open. | open |
| P37 | high | V1 | new | `ambre-toile/src/logic/state/export.rs:56` (`decoded_thumbnail`); `ambre-toile/src/logic/interop/pdf_export.rs:578` | A member writes a media entry whose thumbnail is a valid PNG of 11 000 by 11 000 pixels. The file is small. Each reader decodes it whole on the Logic thread once the element shows. The decode asks for about 480 MiB. | open |
| P38 | medium | V1 | new | `ambre-server/src/redis_ops.rs:91`; `ambre-server/src/routes/share.rs:284`; `ambre-server/src/ws/handler/message.rs:237` | A member leaves a private channel. The relay marks the rotation debt on the `meta` alone. The messages land in the active segment, another document, whose debt mark is never raised. So the relay accepts each frame of the segment under the key the removed member holds. | open |
| P39 | medium | V1 | regression: `N9` reads fixed, and one path keeps the route | `ambre-server/src/ws/handler/message.rs:878` (`membership_still_valid`), `:919` (`sweep_revoked`); `ambre-server/src/ws/handler/mod.rs:304` | A removed member makes its socket loop wait past one sweep, by reading nothing. A frame of its own then finds the membership stale, and the refusal drops the subscription of the connection. The refusal does not drop the route of the hub. The sweep reads the subscriptions alone, so it never sees that document again. The socket keeps each broadcast of the document while it stays open. | open |
| P40 | medium | V1 | known in part: `N26` states the window for a media blob | `ambre-server/src/s3.rs:333`; `ambre-server/src/routes/sync.rs:86`, `:228`, `:386` | A presigned upload link binds the size and no content, for 15 minutes. An editor stores a snapshot, and another member confirms it. The editor then puts other bytes of the same size through the same link. Each later open receives those bytes under the confirmed label. A member removed in that window does the same. | open |
| P41 | medium | V1 | new | `ambre-server/src/s3.rs:405`, `:477`; `ambre-server/src/routes/media.rs:218`; `ambre-server/src/error.rs:197` | A store failure builds an error whose text holds the object key. That key names the document and the media address. The relay logs the error at the error level. The gate of `ZK-030` reads the type of each logged value, and a text hides the identifier from it. | open |
| P42 | medium | V1 | new on the relay; `N25` fixed the SFU side | `ambre-server/src/redis_ops.rs:877` (`join_call`); `ambre-server/src/ws/handler/call.rs:103` | One member opens fifty call sockets, and each other member reads that the call is full. The member also joins and leaves in a loop. The sender index never goes back, so the call refuses every join past 16 384. | open |
| P43 | medium | V1 | known: the limit of `N23` | `ambre-sync/src/updates.rs:1059` (`opened_from_members`); `ambre-sync/src/key_vouch.rs:367` (`check`); `ambre-sync/src/local_store.rs` | A client saw the revoke of Eve and installed the fresh key, then closed. No log head and no key identifier outlive the session. At the next open, the relay serves the log cut before the revoke, and the older genuine wrap. The checks of an open read the cut log alone and install the older key. | open |
| P44 | medium | V1 | new | `ambre-sync/src/sharing.rs:515`, `:532`, `:617` (`invite_task`); `ambre-crypto/src/hash.rs:31` (`fingerprint`) | The pin of an invitee covers its exchange key alone. The relay serves the true exchange key of Bob and a signing key of its own. The inviter signs an entry that grants the writing role to that key. A holder of the document key then seals deltas under that key, and each one reads as a delta of Bob. | open |
| P45 | medium | V1 | new | `ambre-sync/src/documents.rs:282`, `:367`, `:403`; `ambre-sync/src/pipeline.rs:1111` (`open_object`) | A user renames a document that is not open. The client reads the members and unwraps its copy against the wrapper the relay names, with no check of an open. It seals the new name under that key. The relay serves a wrap of a key of its own and reads the name. The listing unwraps the same way, so the relay also shows a name it chose. | open |
| P46 | medium | V1 | known: `P1` by decision, and `P21` | `ambre-sync/src/membership_log.rs:102`, `:654`; `ambre-sync/src/lib.rs:1200`, `:1245`; `ambre-sync/src/chain.rs:116` | A revoked writer seals a fresh delta under a key from before its removal. The relay forwards it. The signer still sits in the writers the log ever granted, so each client applies it. | open |
| P47 | medium | V1 | new | `ambre-sfu/src/serve.rs:328` (`upgrade`), `:358` (`session`), `:186` | The signalling socket of the SFU sets no message bound, and an admitted socket waits for its offer with no deadline. One admission opens many sockets during its 60 s. Each socket holds one unfinished message of the default bound. The SFU process runs out of memory, and every call drops. | open |
| P48 | medium | V1 | new | `ambre-toile/src/logic/state/groups.rs:755` (`recompute_groups`), `:674`, `:690` | An editor writes fifty thousand groups, each one the child of the next. Each cache rebuild and each move costs a quadratic number of steps. A select of the top group recurses once per level and overflows the stack. | open |
| P49 | medium | V1 | new | `ambre-toile/src/message/mod.rs:492` (`LinkTarget::from_bytes`); `ambre-toile/src/logic/state/derive.rs:365`; `ambre-app/src/host/mod.rs:92` (`open_address`) | A member writes the link register of an element with an address that is not `https`. Toile checks the scheme at the local entry alone. The host opens the raw address in a new window, with `noopener`. A handler of an external protocol starts on the click of another member. Prose and Murmure check the scheme at the read. | open |
| P50 | medium | V1 | new | `ambre-app/src/platform.rs:895`, `:950`, `:969`; `ambre-logic/src/persistence.rs:569`; `ambre-logic/src/data.rs:897` | A clear `.ambre` file carries a document identifier in its header, signed by nobody. The file names a shared document of the victim. The open replaces the local snapshot row of that document and deletes its local deltas. The edits the victim made offline are lost, and the next open imports the forged snapshot with no author check. | open |
| P51 | medium | V1 | new | `ambre-trame/src/logic/xlsx.rs:362` (`import_xlsx`); `ambre-trame/src/logic/sheets.rs:411`; `ambre-xlsx/src/ods/read.rs:527`; `ambre-xlsx/src/read.rs:95` | A spreadsheet file declares one hundred thousand empty sheets in a few kilobytes. No reader bounds the count. The import runs on the Logic thread in one call, and the search of a free name reads every sheet for each sheet. | open |
| P52 | medium | V1 | new | `ambre-convert/src/engine/html.rs:36` (`DEPTH_MAX`), `:56`, `:139` | The depth guard of an HTML paste counts with the tokenizer alone. A misnested `<b><div></b>`, or a run of `<rt>`, adds a real level and no counted level. The tree builder then nests past the bound, at a quadratic cost. | open |
| P53 | medium | V1 | new | `ambre-xlsx/src/read.rs:689` (`value_of`); `ambre-xlsx/src/ods/read.rs:850`, `:949`; `ambre-xlsx/src/lib.rs:43` | The cell cap of an import counts cells, not bytes. One shared string of 100 bytes and one million cells that name it give 100 MB of model. A repeated row of an OpenDocument file does the same from one kilobyte. The Main instance then copies and decodes that answer. | open |
| P54 | low | V1 | new | `ambre-server/src/ws/handler/mod.rs:70`, `:303`; `ambre-server/src/ws/handler/call.rs:103` | A socket reads the expiry of its access token at the first frame alone. A stolen token opens a socket that outlives the token, the sign-out and the revoked refresh token. It receives sealed frames and metadata, and writes nothing. | open |
| P55 | low | V1 | known: `P4` keeps the route open; the bucket is new | `ambre-server/src/routes/documents.rs:68` (`setup_account`); `ambre-server/src/middleware/rate_limit.rs:140` | The sign-in tier counts by user once a valid token comes with the request. Each account that an attacker creates brings a fresh bucket. So the creation rate from one address grows with the count of accounts. | open |
| P56 | low | V1 | known: `P8` | `ambre-server/src/routes/orgs.rs:300`, `:421`; `ambre-server/src/db/orgs.rs:491` | The escrow column sits on the user, not on the organization. An owner adds any user and asks the recovery route for that user. Nothing writes the column today. | open |
| P57 | low | V1 | new | `ambre-sync/src/key_vouch.rs:160` | Under a space, a wrap from an exchange key that no row carries reads as a wrap of the space. No check compares that key with the writers of the space. A removed member wraps an older key it holds again, which helps the roll back of `P43`. | open |
| P58 | low | V1 | new, beside `N17` | `ambre-sync/src/membership_log.rs:628` (`apply_act`), `:824` | An editor signs an invite of a third user under the signing key of Alice. The latest binding of a key wins. The directory then drops the row of Alice, so her deltas resolve to no author. | open |
| P59 | low | V1 | known: `P25` and `N13` | `ambre-sync/src/pipeline.rs:447` (`aad_of`); `ambre-crypto/src/ecdh.rs:221` | The additional data of an update, a snapshot and an ephemeral carry no domain of their own. A wrap binds no document and no key. The checks of an open hold both today; the paths of `P26` and `P45` run none. | open |
| P60 | low | V1 | known: `P9`, by design | `ambre-sfu/src/turn.rs:14`; `ambre-sfu/src/serve.rs:372` | A TURN credential lives 12 hours, binds the pseudonym alone and is issued before any seat. A member removed one minute later keeps the relay path for that time. | open |
| P61 | low | V1 | new | `ambre-app/src/call.rs:1398`, `:1404` | The page ignores a failed attach of a receive transform and still plays the track. Without the transform, the decoder reads what the SFU sends. No path to force that failure was found. | open |
| P62 | low | V1 | new | `ambre-sfu/src/forward.rs:617` (`receive`), `:665` | Each datagram from an unknown source scans every member on the one task of the SFU. A seated participant that sends from many sources adds one route per source. A route leaves only when its member does. | open |
| P63 | low | V1 | new | `ambre-toile/src/message/mod.rs:2634`; `ambre-toile/src/ui/presence.rs:304`; `ambre-prose/src/logic/presence.rs:135` | A presence sample carries a display name that its sender fills. The cursor label shows that name. Mallory shows her cursor under the name of Alice. The signature still binds the sample to Mallory. | open |
| P64 | low | V1 | new | `ambre-app/src/host/web_embed.rs:83` (`WebFrames::sync`) | An editor places a web embed. The browser of each viewer loads the page once the element shows, with no gesture. The host of that page learns the address and the time of each viewer. The boundary table names no such leak. | open |
| P65 | low | V1 | new | `ambre-convert/src/engine/webp.rs:23` (`MAX_EDGE`); `ambre-toile/src/logic/state/file_jobs.rs:71` | A lossless WebP of 8 192 by 8 192 pixels weighs a few hundred bytes. The module answers its 256 MiB of pixels whole, and the Main instance copies them twice. The other picture formats answer at the size asked. | open |
| P66 | low | V1 | new | `ambre-odt/src/read.rs:345` (`chain`); `ambre-xlsx/src/ods/styles.rs:165`; `ambre-logic/src/converter_thread.rs:130` | The style chain of an OpenDocument text has no length bound, and two style loops are quadratic. A file that holds a module past its deadline loses that converter thread. A lost thread is never ended, and the fourth loss leaves the session with no converter. | open |

## The overlap with the earlier passes

Counted on 2026-11-17, from the table of open findings.

| Scopes | Findings | New | Known | Regression |
| :-- | :-- | :-- | :-- | :-- |
| the relay, the keys and the calls, which three passes audited | 26 | 13 | 10 | 3 |
| the parsers and the client, which no pass audited | 15 | 15 | 0 | 0 |
| the whole workspace | 41 | 28 | 10 | 3 |

By severity: 12 high, 16 medium and 13 low. Seven high findings are of the first milestone: `P26`
to `P32`.

On the scopes of the earlier passes, half of the blind findings were new. Two of the new ones are
high: `P26` and `P28`. Three findings that read fixed still hold on one path: `N7` (`P32`), `N9`
(`P39`) and `P7` (`P31`). Each fix closed the path its test drives, and left a second path to the
same state.

`P26` is the gravest. The checks of an open run at the open and on a fetched rotation alone. Each
path that writes members or keys reads the members again and trusts the fresh answer. `N15`, `N18`
and `N27` each closed one such path. The defect is the shape, not the path.

## Checked and found sound

The nonces of each seal, and the counter of each chunk key. The order of the open, the strict
verifiers and the commitment tag. The KEK derivation, which refuses a low-order point. The bounded
decompression. The join signature and the wraps of a call. The admission of the SFU. The access
token, the refresh rotation and the OIDC flow. The role clamp of an invite. The local store, sealed
row by row. The UI thread, which receives no key and no token. The package layer of the office
formats, with its bounds on a hostile archive. The formula parser bounds, and the absence of a
formula with a network effect.

## Not covered

The Loro import on forged bytes, whose source this pass could not read. The str0m stack of the
SFU. The Murmure author check per operation, read by samples. `forward.rs` lines 680 to 1120 of the
SFU, and its congestion control. The hot tail, the compaction and the flusher of the relay. The
clipboard, the launcher, the profile and the settings. What stays on disk after a sign-out.
