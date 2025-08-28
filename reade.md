node:
117,46,0,0
[9, 0, 0, 0, 41, 0, 0, 0, 117, 46 0, 0, 1]
[9, 0, 0, 0, 41, 0, 0, 0, 114, 46, 0, 0, 1]

rust:
114, 46, 0, 0

node:

[
61, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0,
140, 25, 133, 92, 45, 0, 0, 0, 64, 64, 114, 116,
105, 106, 119, 92, 65, 70, 82, 79, 32, 72, 79, 85,
83, 69, 92, 83, 117, 112, 101, 114, 32, 70, 108, 117,
32, 45, 32, 66, 101, 108, 105, 101, 118, 101, 32, 53,
88, 46, 109, 112, 51
]

rust
[61, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 148, 99, 2, 0, 45, 0, 0, 0, 64, 64, 114, 116, 105, 106, 119, 92, 65, 70, 82, 79, 32, 72, 79, 85, 83, 69, 92, 83, 117, 112, 101, 114, 32, 70, 108, 117, 32, 45, 32, 66, 101, 108, 105, 101, 118, 101, 32, 53, 88, 46, 109, 112, 51]
I’m building a Soulseek Rust client in the soulseek_download folder. I’m currently developing an MVP for downloading a file. Everything works up to the point where I expect the peer I want to download a file from to respond to my TransferResponse message with a ConnectToPeer message, specifying connectionType F. The ConnectToPeer with connectionType F is never sent to the server.
Here is a log:

❯ cargo run
Compiling soulseek-rs v0.1.0 (/Users/micheldegraaf/src/rust/soulseek_download)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.12s
Running `target/debug/soulseek-rs`
[2025-08-16 22:05:20.139] [DEBUG] ThreadPool initialized with 50 threads
[2025-08-16 22:05:20.139] [INFO] Starting peer listener on port 2234
[2025-08-16 22:05:20.144] [INFO] Connecting to server at server.slsknet.org:2242
[2025-08-16 22:05:20.248] [INFO] Connected to server at server.slsknet.org:2242
[2025-08-16 22:05:20.248] [INFO] Logging in as insane_in_the_brain3
[2025-08-16 22:05:20.352] [INFO] Login successful
[2025-08-16 22:05:20.352] [DEBUG] Server greeting: ""
[2025-08-16 22:05:20.352] [INFO] Logged in as insane_in_the_brain3
[2025-08-16 22:05:20.352] [INFO] Searching for Super flu Believe
[2025-08-16 22:05:20.356] [DEBUG] Parent min speed: 1
[2025-08-16 22:05:20.356] [DEBUG] Parent speed ratio: 50
[2025-08-16 22:05:20.356] [DEBUG] Wishlist search interval: 720 in seconds
[2025-08-16 22:05:20.564] [DEBUG] Number of privileged users: 2743
[2025-08-16 22:05:20.994] [DEBUG] Excluded search phrases: ["bryan adams", "from zero", "housezzz", "village people"]
[2025-08-16 22:05:40.352] [DEBUG] [default_peer:MisterDanielson] OUTGOING RAW: [81, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 80, 102, 209, 7, 65, 0, 0, 0, 64, 64, 97, 120, 110, 115, 111, 92, 77, 117, 115, 105, 99, 92, 83, 111, 117, 108, 83, 101, 101, 107, 92, 53, 48, 46, 32, 83, 117, 112, 101, 114, 32, 70, 108, 117, 32, 45, 32, 66, 101, 108, 105, 101, 118, 101, 32, 40, 69, 120, 116, 101, 110, 100, 101, 100, 32, 77, 105, 120, 41, 46, 109, 112, 51]
[2025-08-16 22:05:40.371] [DEBUG] [default_peer:MisterDanielson] INCOMING RAW (code 41): [19, 0, 0, 0, 41, 0, 0, 0, 80, 102, 209, 7, 0, 6, 0, 0, 0, 81, 117, 101, 117, 101, 100]
[2025-08-16 22:05:40.371] [DEBUG] [default_peer:MisterDanielson] transfer response token: [80, 102, 209, 7] allowed: false
[2025-08-16 22:05:40.371] [DEBUG] [default_peer:MisterDanielson] Transfer rejected: Queued - token [80, 102, 209, 7], I will receive TransferRequest soon...
[2025-08-16 22:06:17.878] [DEBUG] [default_peer:MisterDanielson] INCOMING RAW (code 40): [89, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0, 187, 31, 0, 0, 65, 0, 0, 0, 64, 64, 97, 120, 110, 115, 111, 92, 77, 117, 115, 105, 99, 92, 83, 111, 117, 108, 83, 101, 101, 107, 92, 53, 48, 46, 32, 83, 117, 112, 101, 114, 32, 70, 108, 117, 32, 45, 32, 66, 101, 108, 105, 101, 118, 101, 32, 40, 69, 120, 116, 101, 110, 100, 101, 100, 32, 77, 105, 120, 41, 46, 109, 112, 51, 146, 67, 12, 1, 0, 0, 0, 0]
[2025-08-16 22:06:17.878] [DEBUG] [default_peer:MisterDanielson] TransferRequest for [187, 31, 0, 0]
[2025-08-16 22:06:18.082] [DEBUG] [default_peer:MisterDanielson] Sent TransferResponse for token [187, 31, 0, 0], [9, 0, 0, 0, 41, 0, 0, 0, 187, 31, 0, 0, 1]
[2025-08-16 22:06:18.083] [DEBUG] [default_peer:MisterDanielson] OUTGOING RAW: [9, 0, 0, 0, 41, 0, 0, 0, 187, 31, 0, 0, 1]

Here is the node.js log

[{
user: 'MisterDanielson',
file: '@@axnso\\Music\\SoulSeek\\50. Super Flu - Believe (Extended Mix).mp3',
size: 17580946,
slots: true,
bitrate: 320,
speed: 1158908
}
downloading {"user":"MisterDanielson","file":"@@axnso\\Music\\SoulSeek\\50. Super Flu - Believe (Extended Mix).mp3","size":17580946,"slots":true,"bitrate":320,"speed":1158908}
MisterDanielson transferRequest 81,0,0,0,40,0,0,0,0,0,0,0,194,249,46,187,65,0,0,0,64,64,97,120,110,115,111,92,77,117,115,105,99,92,83,111,117,108,83,101,101,107,92,53,48,46,32,83,117,112,101,114,32,70,108,117,32,45,32,66,101,108,105,101,118,101,32,40,69,120,116,101,110,100,101,100,32,77,105,120,41,46,109,112,51
MisterDanielson recv TransferResponse token: 194,249,46,187 allowed: 0
reason: Queued, token: 194,249,46,187 I will receive TransferRequest soon...
MisterDanielson recv TransferRequest @@axnso\Music\SoulSeek\50. Super Flu - Believe (Extended Mix).mp3 - token: 188,31,0,0
MisterDanielson sending TransferResponse for token 188,31,0,0
[
9, 0, 0, 0, 41, 0,
0, 0, 188, 31, 0, 0,
1
]
connectToPeer MisterDanielson 5.68.182.63 56253 fb300100 F, 251,48,1,0

In the rust version, i don’t get the connectToPeer MisterDanielson with connectionType F

Let’s create an integration test that simulates the server and peer
