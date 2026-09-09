# Soulseek protocol coverage

What this client speaks, and where each message is proven to work. "e2e" means
a test in `soulseek-rs-lib/tests/e2e.rs` drives it against a real server
(soulfind) or a real peer; "unit" means the encoder or parser is tested on its
own, which is all some messages can have — a server we do not run may never
send them.

Message names and codes follow the Soulseek protocol as implemented by
[soulfind](https://github.com/soulfind-dev/soulfind) and
[Nicotine+](https://github.com/nicotine-plus/nicotine-plus).

## Server messages

| Code | Message | State | Covered by |
| --- | --- | --- | --- |
| 1 | Login | yes | e2e — login, wrong password, re-login |
| 2 | SetWaitPort | yes | e2e — the port a listening client reports |
| 3 | GetPeerAddress | yes | e2e — direct download, browse |
| 5 | WatchUser | yes | e2e — watching a user, and an unknown one |
| 6 | UnwatchUser | yes | e2e — updates stop after unwatching |
| 7 | GetUserStatus | yes | e2e — status and share counts, away status |
| 13 | SayChatroom | yes | e2e — a message between two users in a room |
| 14 | JoinRoom | yes | e2e — public rooms, and private ones |
| 15 | LeaveRoom | yes | e2e — room list and membership |
| 16 | UserJoinedRoom | yes | e2e — a member arriving is announced |
| 17 | UserLeftRoom | yes | e2e — a member leaving is announced |
| 18 | ConnectToPeer | yes | e2e — firewalled download and browse |
| 22 | MessageUser | yes | e2e — a private message between users |
| 23 | MessageAcked | yes | e2e — an offline message is delivered once |
| 26 | FileSearch | yes | e2e — search round trip, two clients |
| 28 | SetStatus | yes | e2e — going away is visible to another user |
| 32 | ServerPing | yes | e2e — the session survives pings |
| 35 | SharedFoldersFiles | yes | e2e — share counts as another user sees them |
| 36 | GetUserStats | yes | e2e — stats, and our own recorded speed |
| 41 | Relogged | yes | e2e — a second login reports the first lost |
| 42 | UserSearch | yes | e2e — the named user answers |
| 51 / 52 | AddThingILike / Remove | yes | e2e — interests and similar users |
| 54 | GetRecommendations | yes | e2e — recommendations from our interests |
| 56 | GlobalRecommendations | yes | e2e — the server-wide list |
| 57 | UserInterests | yes | e2e — another user's likes and hates |
| 64 | RoomList | yes | e2e — a joined room appears in the list |
| 66 | AdminMessage | yes | e2e — a server announcement |
| 69 | PrivilegedUsers | yes | e2e — a privileged peer overtakes in the queue |
| 71 | HaveNoParent | yes | e2e — a leaf adopting a parent |
| 83 / 84 | ParentMinSpeed / ParentSpeedRatio | yes | unit — they set the child limit |
| 92 | CheckPrivileges | yes | e2e — how much privilege time we have |
| 93 | EmbeddedMessage | yes | unit — soulfind never sends one |
| 100 | AcceptChildren | yes | e2e — serving children |
| 102 | PossibleParents | yes | e2e — a leaf adopting a parent |
| 103 | WishlistSearch | yes | e2e — a wish is answered like any search |
| 104 | WishlistInterval | yes | e2e — the announced interval is kept |
| 110 | SimilarUsers | yes | e2e — shared interests make users similar |
| 111 | ItemRecommendations | yes | e2e — what else people who like this like |
| 112 | ItemSimilarUsers | yes | e2e — who likes an item |
| 113 / 114 / 115 | RoomTickers and changes | yes | e2e — the board on join, and updates |
| 116 | SetRoomTicker | yes | e2e — a ticker reaches other members |
| 117 / 118 | AddThingIHate / Remove | yes | e2e — hates come back in UserInterests |
| 120 | RoomSearch | yes | e2e — a member of the room answers |
| 121 | SendUploadSpeed | yes | e2e — the server records a finished upload |
| 123 | GivePrivileges | yes | e2e — a privileged account hands time over |
| 126 / 127 | BranchLevel / BranchRoot | yes | e2e — leaf adoption, and children |
| 130 | ResetDistributed | yes | unit — a new session starts parentless |
| 133 | RoomMembers | yes | e2e — a private room's roster |
| 134 / 135 | AddRoomMember / Remove | yes | e2e — granted and revoked |
| 136 | CancelRoomMembership | yes | e2e — a member resigns |
| 137 | CancelRoomOwnership | yes | e2e — the owner disbands the room |
| 139 / 140 | RoomMembershipGranted / Revoked | yes | e2e — the guest is told |
| 141 | EnableRoomInvitations | yes | e2e — set before an invitation |
| 142 | ChangePassword | yes | e2e — the next login needs the new one |
| 143 / 144 | AddRoomOperator / Remove | yes | e2e — the guest is promoted |
| 145 / 146 | RoomOperatorshipGranted / Revoked | yes | e2e — the guest is told |
| 147 | CancelRoomOperatorship | yes | unit |
| 148 | RoomOperators | yes | unit |
| 149 | MessageUsers | yes | e2e — one message, two recipients |
| 150 / 151 / 152 | Global room | yes | e2e — the feed starts and stops |
| 160 | ExcludedSearchPhrases | yes | e2e — the list is kept for our replies |
| 1001 | CantConnectToPeer | yes | e2e — a dial we cannot complete reports back |
| 1003 | CantCreateRoom | yes | e2e — a private room owned by someone else |

Not implemented, and deliberately so: the messages the protocol lists as
obsolete or deprecated, which no current client sends and soulfind answers only
to be polite — SendConnectToken (33), UploadSlotsFull (40), SimilarRecommendations
(50), MyRecommendations (55), PlaceInLineRequest (59), GlobalUserList (67),
ParentIP (73), UserPrivileged (122), NotifyPrivileges (124), AckNotifyPrivileges
(125), ChildDepth (129) and RelatedSearch (153).

## Peer messages

| Code | Message | State | Covered by |
| --- | --- | --- | --- |
| 0 | PierceFirewall | yes | e2e — download and browse through the server |
| 1 | PeerInit | yes | e2e — every direct peer test |
| 4 / 5 | GetShareFileList and its reply | yes | e2e — browsing, in both directions |
| 9 | FileSearchResponse | yes | e2e — searching, in both directions |
| 15 / 16 | UserInfoRequest and its reply | yes | e2e — asking a peer about itself, and answering |
| 36 / 37 | FolderContentsRequest and its reply | yes | e2e — one folder of a peer's shares |
| 40 | TransferRequest | yes | e2e — downloads and uploads |
| 41 | TransferResponse | yes | e2e — downloads, cancellations, refusals |
| 43 | QueueUpload | yes | e2e — queueing, slots, privileged peers |
| 44 | PlaceInQueueResponse | yes | e2e — answering, and asking |
| 46 | UploadFailed | yes | e2e — the download fails promptly |
| 50 | UploadDenied | yes | e2e — the download fails promptly |
| 51 | PlaceInQueueRequest | yes | e2e — both directions |

## Distributed messages

| Code | Message | State | Covered by |
| --- | --- | --- | --- |
| 3 | DistribSearch | yes | e2e — a leaf answers, a parent passes it down |
| 4 | DistribBranchLevel | yes | e2e — adoption, and telling children |
| 5 | DistribBranchRoot | yes | e2e — adoption, and telling children |
| 93 | DistribEmbeddedMessage | yes | unit — unwrapped once, as Nicotine+ does |

Serving children is off by default; see `accept_children` in the README.
