use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep, spawn};
use std::time::Duration;

fn write_message(stream: &mut TcpStream, code: u32, data: Vec<u8>) {
    let mut message = Vec::new();
    message.extend_from_slice(&code.to_le_bytes());
    message.extend_from_slice(&data);

    let size = (message.len() as u32).to_le_bytes();
    stream.write_all(&size).expect("Failed to write size");
    stream.write_all(&message).expect("Failed to write message");
    stream.flush().expect("Failed to flush");
}

fn read_message(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut size_buf = [0u8; 4];
    match stream.read_exact(&mut size_buf) {
        Ok(_) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            return None; // no data yet
        }
        Err(e) => panic!("Failed to read size: {:?}", e),
    }
    let size = u32::from_le_bytes(size_buf);

    let mut message = vec![0u8; size as usize];
    match stream.read_exact(&mut message) {
        Ok(_) => Some(message),
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(e) => panic!("Failed to read message: {:?}", e),
    }
}

#[derive(Debug, Clone)]
enum MockMessage {
    SendConnectToPeer {
        username: String,
        conn_type: String,
        token: Vec<u8>,
    },
    SendFileSearchResponse,
    SendTransferRequest {
        token: Vec<u8>,
    },
}

fn mock_server(
    tx: Arc<Mutex<Sender<MockMessage>>>,
    rx: Arc<Mutex<Receiver<MockMessage>>>,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        println!("[Mock Server] Listening on port {}", port);

        for stream in listener.incoming() {
            let mut stream = stream.expect("Failed to accept");
            let mut stream2 = stream.try_clone().unwrap();
            println!("[Mock Server] Client connected");
            println!("[Mock Server] Client connected");

            let tx = tx.clone();
            let rx = rx.clone();

            thread::spawn(move || loop {
                let message =
                    rx.lock().unwrap().recv_timeout(Duration::from_millis(200));
                if let Ok(message) = message {
                    match message {
                        MockMessage::SendConnectToPeer {
                            username,
                            conn_type,
                            token,
                        } => {
                            // Build ConnectToPeer message
                            let mut connect_msg = Vec::new();

                            // Username
                            let username = "MisterDanielson";
                            connect_msg.extend_from_slice(
                                &(username.len() as u32).to_le_bytes(),
                            );
                            connect_msg.extend_from_slice(username.as_bytes());

                            // Connection type
                            let conn_type = "P";
                            connect_msg.extend_from_slice(
                                &(conn_type.len() as u32).to_le_bytes(),
                            );
                            connect_msg.extend_from_slice(conn_type.as_bytes());

                            // IP (127.0.0.1 reversed)
                            connect_msg.extend_from_slice(&[1, 0, 0, 127]);

                            // Port
                            connect_msg
                                .extend_from_slice(&9001u32.to_le_bytes());

                            // Token
                            connect_msg.extend_from_slice(&[1, 2, 3, 4]);

                            // Additional fields
                            connect_msg.push(0); // privileged
                            connect_msg.push(0); // unknown
                            connect_msg.push(0); // obfuscated_port

                            sleep(Duration::from_secs(10));

                            println!(
                                "[Mock Server] 🔴 ⚠ Sending ConnectToPeer to {}, conn_type: {}, token: {:?}",
                                username,
                                conn_type,
                                token
                            );

                            write_message(&mut stream2, 18, connect_msg);
                        }
                        MockMessage::SendFileSearchResponse => {}
                        MockMessage::SendTransferRequest { token } => {}
                    }
                }
            });
            thread::spawn(move || {
                loop {
                    if let Some(msg) = read_message(&mut stream) {
                        if msg.len() < 4 {
                            continue;
                        }

                        let code = u32::from_le_bytes([
                            msg[0], msg[1], msg[2], msg[3],
                        ]);
                        println!(
                            "[Mock Server] Received message code: {}",
                            code
                        );
                        match code {
                            1 => {
                                // Login - respond with successx
                                println!(
                                    "[Mock Server] Login request received"
                                );
                                let mut response = Vec::new();
                                response.push(1); // success
                                response.extend_from_slice(&0u32.to_le_bytes()); // empty greeting
                                write_message(&mut stream, 1, response);
                                println!("[Mock Server] Sent login success");
                            }
                            2 => {
                                // SetWaitPort - ignore
                                println!("[Mock Server] SetWaitPort received");
                            }
                            35 => {
                                // SharedFoldersFiles - ignore
                                println!(
                                    "[Mock Server] SharedFoldersFiles received"
                                );
                            }
                            71 => {
                                // HaveNoParents - ignore
                                println!(
                                    "[Mock Server] HaveNoParents received"
                                );
                            }
                            28 => {
                                // SetStatus - ignore
                                println!("[Mock Server] SetStatus received");
                            }
                            26 => {
                                // FileSearch - send ConnectToPeer
                                println!("[Mock Server] FileSearch received");

                                // Signal peer to send FileSearchResponse first
                                tx.lock()
                                    .unwrap()
                                    .send(MockMessage::SendFileSearchResponse)
                                    .unwrap();

                                // Wait a bit for peer to process
                                thread::sleep(Duration::from_millis(100));

                                // Build ConnectToPeer message
                                let mut connect_msg = Vec::new();

                                // Username
                                let username = "MisterDanielson";
                                connect_msg.extend_from_slice(
                                    &(username.len() as u32).to_le_bytes(),
                                );
                                connect_msg
                                    .extend_from_slice(username.as_bytes());

                                // Connection type
                                let conn_type = "P";
                                connect_msg.extend_from_slice(
                                    &(conn_type.len() as u32).to_le_bytes(),
                                );
                                connect_msg
                                    .extend_from_slice(conn_type.as_bytes());

                                // IP (127.0.0.1 reversed)
                                connect_msg.extend_from_slice(&[1, 0, 0, 127]);

                                // Port
                                connect_msg
                                    .extend_from_slice(&9001u32.to_le_bytes());

                                // Token
                                connect_msg.extend_from_slice(&[1, 2, 3, 4]);

                                // Additional fields
                                connect_msg.push(0); // privileged
                                connect_msg.push(0); // unknown
                                connect_msg.push(0); // obfuscated_port

                                write_message(&mut stream, 18, connect_msg);

                                // Signal peer about the connection
                                tx.lock()
                                    .unwrap()
                                    .send(MockMessage::SendConnectToPeer {
                                        username: username.to_string(),
                                        conn_type: "P".to_string(),
                                        token: vec![1, 2, 3, 4],
                                    })
                                    .unwrap();
                            }
                            3 => {
                                println!(
                                    "[Mock Server] GetPeerAddress received"
                                );
                            }
                            _ => {
                                println!(
                                    "[Mock Server] Unknown message code: {}",
                                    code
                                );
                            }
                        }
                    }
                }
            });
        }
    });

    port
}

// Minimal mock peer that accepts connections and handles transfers
fn mock_peer(
    rx: Arc<Mutex<Receiver<MockMessage>>>,
    tx: Arc<Mutex<Sender<MockMessage>>>,
) {
    let listener =
        TcpListener::bind("127.0.0.1:9001").expect("Failed to bind peer");

    thread::spawn(move || {
        println!("[Mock Peer] MisterDanielson listening on port 9001");

        for stream in listener.incoming() {
            println!("[Mock Peer] Connection received!");
            let mut stream = stream.expect("Failed to accept peer connection");
            let rx = rx.clone();
            let tx = tx.clone();

            thread::spawn(move || {
                // Set timeout for reads
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .ok();

                // Read initial handshake (9 bytes)
                let mut handshake = [0u8; 9];
                match stream.read_exact(&mut handshake) {
                    Ok(_) => println!("[Mock Peer] Received handshake"),
                    Err(e) => {
                        println!("[Mock Peer] Failed to read handshake: {}", e);
                        return;
                    }
                }

                // Wait for signal from server to send FileSearchResponse
                let mut should_send_response = false;

                // Check if we should send FileSearchResponse based on channel message
                if let Ok(rx_lock) = rx.lock() {
                    while let Ok(msg) = rx_lock.try_recv() {
                        if let MockMessage::SendFileSearchResponse = msg {
                            should_send_response = true;
                            println!("[Mock Peer] Received signal to send FileSearchResponse");
                        }
                    }
                }

                if should_send_response {
                    // Send a minimal FileSearchResponse
                    println!("[Mock Peer] Sending FileSearchResponse");

                    // Build a simple FileSearchResponse (code 9)
                    // This would normally be compressed, but for testing we'll send uncompressed
                    let mut response = Vec::new();

                    // For simplicity, send an empty compressed response
                    // In real implementation this would be zlib compressed search results
                    response.extend_from_slice(&[0, 0, 0, 0]); // empty compressed data

                    write_message(&mut stream, 9, response);
                    println!("[Mock Peer] Sent FileSearchResponse");
                }

                // Now listen for TransferRequest messages
                loop {
                    let mut size_buf = [0u8; 4];
                    if stream.read_exact(&mut size_buf).is_err() {
                        break;
                    }
                    let size = u32::from_le_bytes(size_buf);

                    let mut msg = vec![0u8; size as usize];
                    if stream.read_exact(&mut msg).is_err() {
                        break;
                    }

                    if msg.len() < 4 {
                        continue;
                    }

                    let code =
                        u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);
                    println!("[Mock Peer] Received message code: {}", code);

                    match code {
                        40 => {
                            // TransferRequest
                            println!("[Mock Peer] Received TransferRequest");

                            // Extract token from message
                            let token = vec![msg[8], msg[9], msg[10], msg[11]];
                            println!("[Mock Peer] Token: {:?}", token);

                            // Send TransferResponse with "Queued"
                            let mut response = Vec::new();
                            response.extend_from_slice(&token);
                            response.push(0); // not allowed
                            response.extend_from_slice(&6u32.to_le_bytes()); // "Queued" length
                            response.extend_from_slice(b"Queued");

                            write_message(&mut stream, 41, response);
                            println!(
                                "[Mock Peer] Sent TransferResponse: Queued"
                            );

                            // Check if we should send our own TransferRequest
                            if let Ok(rx_lock) = rx.lock() {
                                while let Ok(msg) = rx_lock.try_recv() {
                                    if let MockMessage::SendTransferRequest {
                                        ..
                                    } = msg
                                    {
                                        println!("[Mock Peer] Received signal to send TransferRequest");
                                    }
                                }
                            }

                            // Always send our TransferRequest after receiving one
                            if true {
                                // Send our own TransferRequest after a short delay
                                thread::sleep(Duration::from_millis(50));

                                let mut request = Vec::new();
                                request.extend_from_slice(&1u32.to_le_bytes()); // direction = 1
                                request.extend_from_slice(&[187, 31, 0, 0]); // peer's token

                                // Filename - using forward slashes
                                let filename = r"@@axnso/Music/SoulSeek/50. Super Flu - Believe (Extended Mix).mp3";
                                request.extend_from_slice(
                                    &(filename.len() as u32).to_le_bytes(),
                                );
                                request.extend_from_slice(filename.as_bytes());

                                // File size
                                request.extend_from_slice(
                                    &17580946u64.to_le_bytes(),
                                );

                                write_message(&mut stream, 40, request);
                                println!("[Mock Peer] Sent TransferRequest with token [187, 31, 0, 0]");
                            }
                        }
                        41 => {
                            // TransferResponse
                            println!("[Mock Peer] Received TransferResponse");
                            let token = vec![msg[4], msg[5], msg[6], msg[7]];
                            let allowed = msg[8];
                            println!(
                                "[Mock Peer] Token: {:?}, Allowed: {}",
                                token, allowed
                            );

                            if allowed == 1 && token == vec![187, 31, 0, 0] {
                                println!(
                                    "sending ConnectionType F from mock_peer"
                                );
                                tx.lock()
                                    .unwrap()
                                    .send(MockMessage::SendConnectToPeer {
                                        username: "MisterDanielson".to_string(),
                                        conn_type: "F".to_string(),
                                        token: vec![187, 31, 0, 0],
                                    })
                                    .unwrap();
                            }
                        }
                        _ => {
                            println!(
                                "[Mock Peer] Unknown message code: {}",
                                code
                            );
                        }
                    }
                }
            });
        }
    });
}

#[test]
fn test_simple_connect_to_peer() {
    use soulseek_rs::{Client, PeerAddress};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Set up a 5-second timeout for the entire test
    let test_complete = Arc::new(AtomicBool::new(false));
    let test_complete_clone = test_complete.clone();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(120));
        if !test_complete_clone.load(Ordering::Relaxed) {
            println!("\n\n[ERROR] Test timeout after 5 seconds!");
            std::process::exit(1);
        }
    });
    let (tx, rx) = channel::<MockMessage>(); // shared channel

    let tx = Arc::new(Mutex::new(tx));
    let rx = Arc::new(Mutex::new(rx));

    let server_port = mock_server(tx.clone(), rx.clone());
    thread::sleep(Duration::from_millis(50));
    // Start mock peer
    mock_peer(rx, tx.clone());
    thread::sleep(Duration::from_millis(50));

    // Create and connect client
    let mut client = Client::new(
        PeerAddress::new(String::from("127.0.0.1"), server_port),
        String::from("test_user"),
        String::from("test_pass"),
    );

    println!("Connecting to mock server on port {}", server_port);
    client.connect();

    // Login
    println!("Logging in...");
    client.login().expect("Login failed");

    // Wait for login to complete
    thread::sleep(Duration::from_millis(50));

    // Search - this should trigger ConnectToPeer
    println!("Searching...");
    let _ = client.search("test query", Duration::from_millis(200));

    // Wait for peer connection to be established
    thread::sleep(Duration::from_millis(200));

    // Now test download
    println!("\n=== Starting Download Test ===");
    let filename =
        r"@@axnso/Music/SoulSeek/50. Super Flu - Believe (Extended Mix).mp3";
    let result = client.download(
        filename.to_string(),
        "MisterDanielson".to_string(),
        17580946,
    );
    println!("Download result: {:?}", result);

    // Wait a bit to see the full flow
    thread::sleep(Duration::from_secs(50));

    println!("Test completed");

    // Mark test as complete
    test_complete.store(true, Ordering::Relaxed);
}
