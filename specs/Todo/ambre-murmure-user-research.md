# AMBRE -- MURMURE: USER RESEARCH ON TEAMS, SLACK AND DISCORD

**Type:** Working note (evidence for the Murmure sketch)
**Implementation:** design-only

> A working note, and not a specification. It records what users like and dislike in the
> messaging part of Microsoft Teams, Slack and Discord. A second pass adds Zulip for threads and
> Element for end-to-end encrypted (E2EE) messaging. A third pass compares the bot systems and
> their E2EE stakes. Video, calls and file storage are out of scope. It declares no contract row. `ambre-murmure.md` stays the sketch; this note is the
> evidence that the sketch cites.

## 1. METHOD AND LIMITS

* **Date:** read on 2026-09-24, through web searches. The sources are in section 7.
* **Second pass:** read on 2026-09-24. It checks the first pass and adds two products.
  * Zulip: the product that users cite for threads that work.
  * Element: an E2EE client on Matrix. Its failures are the failures that Murmure can have.
* **Sources:** press articles, protocol proposals, issue trackers, review sites (Capterra, G2, Trustpilot), vendor feedback forums and
  vendor help pages. There is no survey and no interview.
* **Bias:** complaints are louder than praise. A review site samples angry users. Treat each item
  as a signal, not as a measure.
* **No figure is a user count.** A figure below is a vendor limit or a vendor claim, with its date.

## 2. FINDINGS PER PRODUCT

### A. Microsoft Teams

| Users like | Users dislike |
| :--- | :--- |
| Chat, meetings and Office files in one tool, bundled with Microsoft 365. | Search: users describe finding an old message or a link as very difficult. |
| The merged "Chat and channels" view (preview from November 2024): filters, favorites, custom sections. | Two messaging models: a chat is flat, a channel post is threaded. Users do not know which one to use. |
| Channel threads that stay attached to the root post and stay searchable. | Channel threads: a new post pushes older posts up and out of sight, whatever the user did with them. |
| The "Threads" channel layout (general availability in August 2025): a channel reads like a chat, and a reply opens a side pane. | Three messaging models since August 2025: a flat chat, a channel in posts layout, a channel in threads layout. |
| | Notifications: too many, and some clear only when the user opens the chat. |
| | Performance: slow start, high memory and CPU use. Microsoft rebuilt the client in 2023 and claimed "2x faster, 50% less memory". |
| | Layout: the chat view uses too much space and works badly in a small window. |

### B. Slack

| Users like | Users dislike |
| :--- | :--- |
| Threads that feel natural: a side discussion does not interrupt the channel. | Threads hide messages: a reply in a thread is easy to miss for channel readers. |
| Speed: the quick switcher, keyboard navigation, `Ctrl+J` to the first unread, `Shift+Esc` to mark all as read. | The 2023 redesign: a permanent left navigation bar that users cannot remove, and unread badges moved deeper into sub-views. |
| Reactions as a light reply ("seen", "agreed", "done") that does not add noise. | History as a hostage: the free plan shows 90 days, and since 2024-08-26 Slack deletes free content older than one year. |
| Slash commands and integrations. | Privacy: in May 2024, users found that Slack trained global models on customer messages by default. The opt-out was an email from the admin. |
| One "Unreads" view that collects every unread message. | The redesign serves users of every feature; users of basic messaging see only extra clutter. |
| | AI features (2025-2026): admins react with doubt, and the AI add-on moved into a more expensive plan in August 2025. |

### C. Discord

| Users like | Users dislike |
| :--- | :--- |
| One channel per topic, and fine-grained roles and permissions that are easy to manage. | Search: users must type exact phrases, results page badly, and a recent update removed filters. |
| Forum channels: each post has a title, tags and its own thread, so the content stays findable. | No search for "messages that reply to me" when the author did not ping the user. |
| Speed and simplicity: markdown, embeds, custom emoji, "Mark Unread" on any message. | Old direct messages are hard to find again. |
| Voice channels that users join with one click. | Threads auto-archive after a fixed delay, which users find too short. |
| | Forum channels on mobile: a forum that requires tags is unusable on Android. |
| | Privacy: the global age verification (ID or face scan) caused a backlash in February 2026. Discord relaunched it on 2026-09-23 with non-biometric options, such as a credit card or an app store age range. |

### D. Zulip (second pass)

| Users like | Users dislike |
| :--- | :--- |
| Each thread has a topic, that is a title, so a thread stays easy to find. | Topics need a change of habit for users that come from Slack. |
| A user can mark a whole topic as read after time away. | |
| A user that joins hours later reads one topic and gets the context. | |

### E. Element on Matrix (second pass)

| Users like | Users dislike |
| :--- | :--- |
| End-to-end encryption, and a server that the organization can host. | "Unable to decrypt": a message shows no content because a key is missing on the device. Element tracks it in a meta issue. |
| Room history shared with new members (MSC3061, then MSC4268 "room key bundle"). | Search in encrypted rooms works on desktop only. A web index is open as a Labs pull request (2026-09), and it covers a recent window. |
| | Notifications at the "mentions and keywords" level fail on mobile in encrypted rooms: the homeserver cannot read a mention. |

## 3. CROSS-CUTTING THEMES

1. **Search is the first complaint on all three products.** Users know the message exists and
   cannot find it.
2. **Threads divide users.** Users want side discussions, but a thread that hides content from the
   channel reader loses information.
3. **Notification fatigue.** Too many notifications, and read state that the user does not control.
4. **Speed is a feature.** Slack wins on keyboard speed; Teams loses on start time and memory.
5. **Clutter from feature creep.** Users of basic messaging reject navigation added for other
   features.
6. **History and privacy are trust issues.** Users react strongly when a vendor deletes history,
   reads messages for training, or asks for identity documents.
7. **E2EE has its own failures.** A missing key hides a message, search stays on one device, and
   the server cannot read a mention to filter a notification.

## 4. DO AND DO NOT FOR MURMURE

| # | Do | Do not | Theme |
| :--- | :--- | :--- | :--- |
| 1 | Make local search fast and complete: filters by author, date, channel, file type, and "replies to me". | Rely on exact-phrase matching. | 1 |
| 2 | Show the index coverage: "searched 2 of 14 segments". Offer to fetch the rest. | Return a silent partial result. | 1 |
| 3 | Keep one model: a thread is a reply chain on any message, in a channel or a direct message. | Keep several models, as Teams does with chat, posts and threads. | 2 |
| 4 | Show a reply count and the last replier under the root message. Offer "also send to channel". | Push old threads out of sight when a new post arrives. | 2 |
| 5 | Offer one "Threads" view with every followed thread that has unread replies. | Auto-archive a thread on a fixed delay. | 2 |
| 6 | Give per-channel notification levels: all, mentions, none. Default a large channel to mentions. | Notify each message of a large channel by default. | 3 |
| 7 | Let the user mark a message as unread, and mark all as read with one key. | Clear an unread state only when the user opens the conversation. | 3 |
| 8 | Sync the read cursor across devices, as the sketch already does. | Keep the read state per device. | 3 |
| 9 | Give full keyboard navigation: quick switcher, jump to first unread, edit the last message. | Require the mouse for a frequent action. | 4 |
| 10 | Hold the start time and memory budgets of `ambre-spec-budgets.md`. | Load a full channel history at start. | 4 |
| 11 | Keep the messaging view minimal. Other applications stay one command away. | Add a permanent navigation bar that the user cannot hide. | 5 |
| 12 | Keep the full history as long as the organization keeps it. Retention is an explicit admin policy. | Delete history to sell a plan. | 6 |
| 13 | Keep the zero-knowledge promise visible: no server-side model can read a message. | Ask for identity documents, or use content for any server-side process. | 6 |
| 14 | Support reactions as a light reply, with custom emoji per space. | Count a reaction as a message in the unread badge. | 3 |
| 15 | Offer a forum-like channel: each post has a title and tags. | Make a forum unusable on mobile. | 2 |
| 16 | Let the user mark a whole thread as read. | Force the user to open each reply. | 3 |
| 17 | Show why a message cannot be decrypted, and what fixes it. | Show a bare "unable to decrypt". | 7 |
| 18 | Give the relay the routing data that a notification level needs. | Rely on a silent push that the client filters. | 7 |

## 5. WHAT THIS RESEARCH CHALLENGES IN THE SKETCH

* **Local search on a new device.** `ambre-murmure.md` section 5 indexes decrypted content on the
  client. A new device holds only the active segment. So search is partial until the client
  fetches and decrypts every sealed segment. Row 2 above is the minimum answer.
* **Pins.** Section 2.B stores pins in local storage, but the `meta` document holds a shared
  `pinned` array. Users expect a pin to be shared with the channel. A personal bookmark is a
  different feature, and it needs a different name.
* **Presence and typing indicators.** Section 2.B shows them through the relay. They leak activity
  timing, which is metadata. Each indicator needs a user setting, and the default needs a decision.
* **Threads in the schema.** The segment stores `reply_to` only. Rows 4 and 5 need a reply count
  and a "followed" state per user. A reply can land in a later segment than its root, so the
  count cannot live in the root segment alone.
* **Notifications.** Section 4 hides message content from the push proxy. Rows 6 and 7 need the
  notification level per channel. Client-side filtering after decryption fails on the web:
  * Chrome, Firefox and Safari require `userVisibleOnly: true` for Web Push.
  * Safari cancels the subscription after three pushes that show no notification.
  * Chrome shows a generic notification for such a push.
  * On iOS, only an entitlement that Apple grants on request lets an app hide a push.

  So the relay needs routing metadata, as in Matrix MSC4028. Row 18 above is the answer.
* **New member onboarding.** A key rotation creates a new Document Key. A new member needs every
  key since the channel start. The Matrix MSC3061 implementation had a vulnerability. Its
  successor sends one signed bundle of keys.
* **Decryption failures.** The sketch does not say what a client shows for a message whose key is
  missing. Element users rank that state as a main complaint. Row 17 above is the minimum answer.

## 6. BOTS AND INTEGRATIONS

Third pass, read on 2026-09-24. It compares how each product lets a program read and post
messages. Section 7 of `ambre-murmure.md` is the sketch that this pass challenges.

### A. Comparison

| Product | Bot model | What a bot can read | Interactive messages | Inbound webhook |
| :--- | :--- | :--- | :--- | :--- |
| Slack | An app with OAuth scopes. Events come by HTTP or Socket Mode. | Scopes per API method. Since 2025-05-29, an app outside the Marketplace reads 15 messages per request, one request per minute. | Block Kit. New blocks for AI agent answers in 2025-2026. | One webhook URL per channel. |
| Microsoft Teams | A bot on the Bot Framework SDK. Microsoft archived the SDK at the end of 2025 for the Microsoft 365 Agents SDK. | Scopes in the app manifest, with admin consent. | Adaptive Cards. | Office 365 connectors retired. Workflows (Power Automate) replace them, and the deadline moved several times up to 2026. |
| Discord | A bot user on the gateway, plus application commands. | Message content is a privileged intent. Without it, a bot reads its direct messages, the messages that mention it and its commands. | Buttons, select menus, modals. | One webhook URL per channel. |
| Zulip | Incoming webhook bots, outgoing webhook bots, generic bots. | An outgoing webhook bot reads its direct messages and the messages that mention it. | Text commands through the Botserver. | One URL per integration. Most official integrations use it. |
| Element on Matrix | A bot is a room member, or an application service. | In an encrypted room, a bot needs the room keys, as a human member does. | Text commands, widgets. | The Hookshot bridge. |
| Wire | An app is a member of the Messaging Layer Security (MLS) group, through the Wire Integrations SDK. | The app decrypts locally with its own key material. | Not read in this pass. | Not read in this pass. |
| WhatsApp Business | The Cloud API, hosted by Meta. The On-Premises API ended on 2025-10-23. | Meta decrypts each message at the API and forwards the plaintext to the business. | Templates and buttons. | Not applicable. |
| Signal | No official bot API, by design. The unofficial `signal-cli` is the common tool. | A bot is a full client and reads every conversation that it is in. | None. | None. |
| Telegram | A bot exists only in cloud chats. A secret chat, the E2EE mode, has no bot. | Telegram servers read every bot conversation. | Inline keyboards. | Not read in this pass. |
| Threema Gateway | Two modes. In basic mode, Threema holds the key. In end-to-end mode, the business holds the private key. | In end-to-end mode, only the business server reads the messages, through an HTTPS callback. | None. | Not applicable. |
| Keybase | A restricted bot is a team member without the team key. | Members derive a key per bot from the team key seed. They encrypt with it only the messages for the bot, for example `!meet`. The bot reads nothing else. | Text commands. | Not read in this pass. |

### B. What users and developers dislike

* **Forced migrations.** Teams retired the Office 365 connectors and archived the Bot Framework SDK.
  Integrators rewrite integrations that work.
* **Two card formats.** A bot for Slack and Teams translates between Block Kit and Adaptive Cards.
  The two JSON schemas do not map one to one.
* **Discord restrictions.** Bot developers resisted the move to slash commands and the privileged
  message content intent. A bot has 100 top-level commands at most.
* **Slack 2025 rate limit.** The limit breaks unvetted apps and AI agents that read history. Slack
  gives bulk data exfiltration as the reason.
* **Matrix bots in encrypted rooms.** A bot ignores a message when a room key is missing. Hookshot
  fails to start after a Synapse change to the MSC3202 device identifier.

### C. E2EE stakes

1. **A key holder reads everything.** Slack, Discord and Zulip filter what a bot reads on the
   server. A blind relay cannot filter content, so the scope of a bot must be cryptographic.
   Keybase restricted bots are the one product in this pass with a cryptographic scope.
2. **History.** Murmure gives the full history to a new member. The same rule gives a bot years of
   messages. Slack throttles history reads by apps to stop exfiltration, without E2EE.
3. **Plaintext lives where the bot runs.** With the WhatsApp Cloud API, E2EE ends at Meta. A bot
   that calls a hosted AI model sends the plaintext to that model provider.
4. **One symmetric key gives read and write together.** An inbound webhook bridge only posts. With
   the Document Key, the bridge can also read the channel. A public HTTP endpoint then holds the
   key of the channel.
5. **A member bot removal costs a rotation.** Murmure rotates the Document Key when a member
   leaves. A bot that holds the Document Key is a member, so its removal rotates the key for every
   member.
6. **The relay still sees metadata.** The relay knows which identity downloads which sealed
   segment. The relay can rate-limit a bot identity, as Slack does on plaintext.
7. **Optional encryption fails.** Matrix bots got E2EE years after the rooms did, and it stays
   fragile. An SDK that makes encryption an option ships bots that fail in encrypted rooms.
8. **A bot message is untrusted input.** The client renders an interactive message with wgpu. A
   remote image in that message leaks the IP address and the read time of each viewer.

### D. Do and do not for Murmure bots

| # | Do | Do not | Stake |
| :--- | :--- | :--- | :--- |
| B1 | Give two bot classes. A restricted bot holds only a bot key, derived from the Document Key. A member bot holds the Document Key. Default to the restricted bot. | Give every bot the Document Key. | 1 |
| B2 | Give a member bot the keys from its join onward. The admin grants history explicitly. | Send a bot the full history bundle by default. | 2 |
| B3 | Give a write-only integration its own posting key. The members hold that key and read the posts with it. | Give an inbound webhook bridge the Document Key. | 4 |
| B4 | Show where a bot runs, and each third party that receives content, before the admin invites the bot. | Hide that a bot sends plaintext to an external AI model. | 3 |
| B5 | Let the relay rate-limit sealed segment downloads per bot identity. | Let a bot download every sealed segment at once, unseen. | 6 |
| B6 | Put encryption inside the SDK. A bot cannot connect without the Murmure crypto. | Offer an SDK where encryption is an option. | 7 |
| B7 | Keep one versioned declarative schema for interactive messages, with no remote content. | Load a remote image or a script from a bot message. | 8 |
| B8 | Keep the bot protocol stable and versioned. | Retire an integration path with a forced migration. | 6.B |
| B9 | Store the slash command registry in the encrypted `meta` document. The client autocompletes commands. | Register commands on the relay. | 1 |

A restricted bot (B1) follows the Keybase model:

1. Each member derives the bot key with HKDF, from the Document Key and the bot identifier.
2. The client encrypts with the bot key only a message for the bot, for example a slash command.
3. The bot answers with the same bot key, so the bot key is also its posting key (B3).
4. A removal rotates no Document Key. The members stop using the bot key of the removed bot.

A key rotation creates a new Document Key, so each member derives a new bot key.

### E. What this challenges in the sketch

* **Section 7.A.** The sketch shares the document session key with each invited bot. Rows B1 and
  B2 replace that rule for most bots.
* **Section 7.C, inbound webhooks.** The "Encryptor" receives plaintext JSON from GitHub or Stripe.
  The organization runs it, never the relay operator. Rows B3 and B4 apply.
* **Section 7.C, ephemeral bots, and section 7.B, WASM applets.** Both run on the client, so no
  plaintext leaves the device. The research supports them as the default for automation.
* **Section 7.D, trust indicator.** The badge certifies a key. It must also show where the bot
  runs (row B4).

## 7. SOURCES

Read on 2026-09-24.

* TechRadar, "Microsoft knows about the worst Teams problems, but can't decide how to fix them":
  <https://www.techradar.com/news/microsoft-teams-could-leave-its-rivals-in-the-dust-but-it-wont>
* Blind, "Microsoft Teams sucks": <https://www.teamblind.com/post/microsoft-teams-sucks-famqmgzf>
* Microsoft Tech Community, "Introducing the new Microsoft Teams chat and channels experience":
  <https://techcommunity.microsoft.com/blog/microsoftteamsblog/introducing-the-new-microsoft-teams-chat-and-channels-experience/4275674>
* Microsoft Adoption, "Threads in Microsoft Teams channels":
  <https://adoption.microsoft.com/en-us/microsoft-teams/new-chat-and-channels-experience/threads-in-channels/>
* Northwest University, "Teams Chat vs Channel Posts": <https://help.northwestu.edu/help/article/312695>
* TechCrunch, "Microsoft rebuilds Teams, promises 2x faster performance" (2023-03-27):
  <https://techcrunch.com/2023/03/27/microsoft-rebuilds-teams-promises-2x-faster-performance/amp/>
* VentureBeat, "Slack defends controversial redesign amid sharp criticism":
  <https://venturebeat.com/virtual/slack-lash-slack-defends-controversial-redesign-amid-sharp-criticism>
* Fast Company, "Four ways Slack's confusing redesign is making it hard to work today":
  <https://www.fastcompany.com/90972862/four-ways-the-confusing-slack-redesign-is-making-it-hard-to-work-today>
* m.io, "Slack Threads: Best Friend Or Arch Enemy?": <https://www.m.io/blog/slack-threads>
* XDA Developers, "7 reasons Slack is better than Teams":
  <https://www.xda-developers.com/7-reasons-slack-is-better-than-teams/>
* Slack Help, "View all your unread messages":
  <https://slack.com/help/articles/226410907-View-all-your-unread-messages>
* Slack Help, "Feature limitations on the free version of Slack":
  <https://slack.com/help/articles/27204752526611-Feature-limitations-on-the-free-version-of-Slack>
* Forbes, "Slack Issues Imminent Mass Content Deletion Warning To Users" (2024-08-19):
  <https://www.forbes.com/sites/jaymcgregor/2024/08/19/slack-delete-files-and-messages-august-26-free-account-premium/>
* TechCrunch, "Slack under attack over sneaky AI training policy" (2024-05-17):
  <https://techcrunch.com/2024/05/17/slack-under-attack-over-sneaky-ai-training-policy/>
* Discord Support, "Forum & Thread Feedback":
  <https://support.discord.com/hc/en-us/community/posts/8176809013271-Forum-Thread-Feedback>
* Discord Support, "Forum Channel Feedback":
  <https://support.discord.com/hc/en-us/community/posts/8953226512663-Forum-Channel-Feedback>
* Discord Support, "Fix the search function":
  <https://support.discord.com/hc/en-us/community/posts/19702613381655-Fix-the-search-function>
* Discord Support, "Add 'replying to:' in the search bar":
  <https://support.discord.com/hc/en-us/community/posts/4415641102999-Add-replying-to-in-the-search-bar-to-search-all-replies-including-those-where-pings-were-disabled>
* Discord Support, "Making old direct messages accessible":
  <https://support.discord.com/hc/en-us/community/posts/360035228491-Making-old-direct-messages-accessible>
* Capterra, "Discord Reviews": <https://www.capterra.com/p/240238/Discord/reviews/>
* TechCrunch, "Discord delays global rollout of age verification after backlash" (2026-02-24):
  <https://techcrunch.com/2026/02/24/discord-delays-global-rollout-of-age-verification-after-backlash/>
* TechCrunch, "Discord's age-verification era is upon us, despite community backlash" (2026-09-22):
  <https://techcrunch.com/2026/09/22/discords-age-verification-era-is-upon-us-despite-community-backlash/>

Second pass, read on 2026-09-24.

* Microsoft 365 Message Center, "MC1088172 -- New threaded conversation layout option for channels":
  <https://mc.merill.net/message/MC1088172>
* Practical365, "Threaded Layout Available for Teams Channels":
  <https://practical365.com/teams-threaded-layout/>
* eesel AI, "Slack AI in 2026: Features, pricing, and limitations":
  <https://www.eesel.ai/blog/slack-ai>
* CyberInsider, "Discord rolls out age checks that don't require an ID or selfie":
  <https://cyberinsider.com/discord-rolls-out-age-checks-that-dont-require-an-id-or-selfie/>
* Hacker News, "Slack threads are useless compared to Zulip":
  <https://news.ycombinator.com/item?id=27149487>
* Zulip, "Why Zulip?": <https://zulip.com/why-zulip/>
* Element, "Unable To Decrypt meta issue": <https://github.com/element-hq/element-meta/issues/245>
* Element Web, pull request 34718, "Add a browser EventIndex":
  <https://github.com/element-hq/element-web/pull/34718>
* Synapse, issue 16562, "No push notifications received for rooms with 'mentions and keywords'":
  <https://github.com/matrix-org/synapse/issues/16562>
* Matrix, MSC3061 "Sharing room keys for past messages":
  <https://github.com/matrix-org/matrix-spec-proposals/pull/3061>
* Matrix, MSC4268 "Encrypted history sharing":
  <https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4268-encrypted-history-sharing.md>
* Matrix, MSC4028 "Push all encrypted events except for muted rooms":
  <https://github.com/matrix-org/matrix-spec-proposals/pull/4028>
* WebKit, "Meet Web Push": <https://webkit.org/blog/12945/meet-web-push/>
* DEV Community, "How to fix iOS push subscriptions getting terminated after 3 notifications":
  <https://dev.to/progressier/how-to-fix-ios-push-subscriptions-being-terminated-after-3-notifications-39a7>
* Apple Developer Forums, "usernotifications.filtering entitlement":
  <https://developer.apple.com/forums/thread/713997>

Third pass (bots), read on 2026-09-24.

* Slack Developer Docs, "Rate limit changes for non-Marketplace apps" (2025-05-29):
  <https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps/>
* Slack Developers, "Build richer agent experiences with Block Kit":
  <https://slack.dev/build-richer-agent-experiences-with-block-kit/>
* Microsoft 365 Developer Blog, "Retirement of Office 365 connectors within Microsoft Teams":
  <https://devblogs.microsoft.com/microsoft365dev/retirement-of-office-365-connectors-within-microsoft-teams/>
* Microsoft 365 Message Center, "MC1181996 -- Migration update for Office 365 connectors retirement":
  <https://mc.merill.net/message/MC1181996>
* Microsoft, "botframework-sdk" repository README (archive notice):
  <https://github.com/microsoft/botframework-sdk>
* Syncrivo, "Slack Block Kit vs Adaptive Cards for Message Formatting":
  <https://syncrivo.ai/en/blog/slack-block-kit-vs-adaptive-cards-cross-platform-formatting>
* Discord Developers, "You Might Not Need a Privileged Intent":
  <https://docs.discord.com/developers/gateway/you-might-not-need-a-privileged-intent>
* Discord API Docs, discussion 3540, "Message Content Access Deprecation for Verified Bots":
  <https://github.com/discord/discord-api-docs/discussions/3540>
* Zulip Help, "Outgoing webhooks": <https://zulip.com/help/outgoing-webhooks>
* Zulip API, "Incoming webhook integrations": <https://docs.zulip.com/api/incoming-webhooks-overview>
* Matrix Hookshot, "Encryption":
  <https://matrix-org.github.io/matrix-hookshot/latest/advanced/encryption.html>
* Matrix Hookshot, issue 1398, "Hookshot fails to start against Synapse 1.161.0":
  <https://github.com/matrix-org/matrix-hookshot/issues/1398>
* matrix-bot-sdk, issue 208, "Encrypted appservices can't decrypt event":
  <https://github.com/turt2live/matrix-bot-sdk/issues/208>
* Wire, "Wire Integrations SDK: Build Secure Apps Inside Encrypted Conversations" (search summary
  only, the page was not reachable):
  <https://wire.com/en/blog/wire-integrations-sdk-secure-encrypted-apps>
* Meta for Developers, "WhatsApp Cloud API -- Data Privacy & Security":
  <https://developers.facebook.com/documentation/business-messaging/whatsapp/data-privacy-and-security/>
* Meta for Developers, "On-Premises API Sunset":
  <https://developers.facebook.com/docs/whatsapp/on-premises/sunset>
* signal-cli, "An unofficial commandline and JSON-RPC interface for Signal":
  <https://github.com/AsamK/signal-cli>
* Telegram, "End-to-End Encryption, Secret Chats": <https://core.telegram.org/api/end-to-end>
* Telegram, "FAQ": <https://telegram.org/faq>
* Threema Gateway, "Products and prices": <https://gateway.threema.ch/en/products>
* Threema Gateway, "API": <https://gateway.threema.ch/en/developer/api>
* Keybase Book, "Restricted Bots in Chat": <https://book.keybase.io/docs/chat/restricted-bots>
* Keybase Docs, "Chat -- Restricted Bots": <https://keybase.io/docs/chat/restricted_bots>
* Keybase Blog, "Introducing Keybase bots": <https://keybase.io/blog/bots>
