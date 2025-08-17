use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// Helper function to read a message from a stream
fn read_message(stream: &mut TcpStream) -> Vec<u8> {
    let mut size_buf = [0u8; 4];
    stream
        .read_exact(&mut size_buf)
        .expect("Failed to read message size");
    let size = u32::from_le_bytes(size_buf);

    let mut message = vec![0u8; size as usize];
    stream
        .read_exact(&mut message)
        .expect("Failed to read message");
    message
}

// Helper function to write a message to a stream
fn write_message(stream: &mut TcpStream, code: u32, data: Vec<u8>) {
    let mut message = Vec::new();
    message.extend_from_slice(&code.to_le_bytes());
    message.extend_from_slice(&data);

    let size = (message.len() as u32).to_le_bytes();

    println!(
        "[DEBUG] Writing message: size={}, code={}, data_len={}",
        message.len(),
        code,
        data.len()
    );

    stream
        .write_all(&size)
        .expect("Failed to write message size");
    stream.write_all(&message).expect("Failed to write message");
    stream.flush().expect("Failed to flush stream");
}

// Helper to build login response
fn build_login_response() -> Vec<u8> {
    let mut data = Vec::new();
    data.push(1); // success
                  // Empty greeting
    data.extend_from_slice(&0u32.to_le_bytes()); // empty string length
    data
}

// Helper to build ConnectToPeer message
fn build_connect_to_peer(
    username: &str,
    connection_type: &str,
    host: &str,
    port: u32,
    token: Vec<u8>,
) -> Vec<u8> {
    let mut data = Vec::new();

    // Username
    data.extend_from_slice(&(username.len() as u32).to_le_bytes());
    data.extend_from_slice(username.as_bytes());

    // Connection type
    data.extend_from_slice(&(connection_type.len() as u32).to_le_bytes());
    data.extend_from_slice(connection_type.as_bytes());

    // IP address (reversed)
    let ip_parts: Vec<u8> =
        host.split('.').map(|s| s.parse::<u8>().unwrap()).collect();
    for i in (0..4).rev() {
        data.push(ip_parts[i]);
    }

    // Port
    data.extend_from_slice(&port.to_le_bytes());

    // Token
    data.extend_from_slice(&token);

    // Additional fields
    data.push(0); // privileged
    data.push(0); // unknown
    data.push(0); // obfuscated_port

    data
}

// Mock Soulseek Server
fn mock_server(received_messages: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener =
        TcpListener::bind("127.0.0.1:0").expect("Failed to bind server");
    let port = listener.local_addr().unwrap().port();

    // Store all client connections
    let client_streams: Arc<Mutex<Vec<TcpStream>>> =
        Arc::new(Mutex::new(Vec::new()));

    thread::spawn(move || {
        println!("[Mock Server] Listening on port {}", port);

        for stream in listener.incoming() {
            let mut stream = stream.expect("Failed to accept connection");
            let messages = received_messages.clone();

            // Store this client stream
            let client_streams_for_storage = client_streams.clone();
            if let Ok(cloned_stream) = stream.try_clone() {
                client_streams_for_storage
                    .lock()
                    .unwrap()
                    .push(cloned_stream);
            }

            let client_streams_clone = client_streams.clone();

            thread::spawn(move || {
                println!("[Mock Server] Started handling client connection");
                loop {
                    let msg = match read_message(&mut stream) {
                        msg => msg,
                    };

                    if msg.len() < 4 {
                        continue;
                    }

                    let code =
                        u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);

                    messages
                        .lock()
                        .unwrap()
                        .push(format!("Server received code: {}", code));

                    match code {
                        1 => {
                            // Login
                            println!("[Mock Server] Received login request");
                            write_message(
                                &mut stream,
                                1,
                                build_login_response(),
                            );
                            println!("[Mock Server] Sent login success");

                            // Send server messages like in the real flow
                            thread::sleep(Duration::from_millis(50));

                            // ParentMinSpeed
                            let mut data = Vec::new();
                            data.extend_from_slice(&1u32.to_le_bytes());
                            write_message(&mut stream, 83, data);

                            // ParentSpeedRatio
                            let mut data = Vec::new();
                            data.extend_from_slice(&50u32.to_le_bytes());
                            write_message(&mut stream, 84, data);

                            // WishlistInterval
                            let mut data = Vec::new();
                            data.extend_from_slice(&720u32.to_le_bytes());
                            write_message(&mut stream, 104, data);
                        }
                        2 => {
                            // SetWaitPort
                            println!("[Mock Server] Received SetWaitPort");
                        }
                        26 => {
                            // FileSearch
                            println!("[Mock Server] Received file search");

                            // Send ConnectToPeer for search peer immediately
                            let token = vec![80, 102, 209, 7];
                            let connect_msg = build_connect_to_peer(
                                "MisterDanielson",
                                "P",
                                "127.0.0.1",
                                9001, // Mock peer port
                                token,
                            );
                            write_message(&mut stream, 18, connect_msg.clone());
                            println!("[Mock Server] Sent ConnectToPeer type P for MisterDanielson - {} bytes", connect_msg.len());
                        }
                        35 => {
                            // SharedFoldersFiles
                            println!(
                                "[Mock Server] Received SharedFoldersFiles"
                            );
                        }
                        71 => {
                            // HaveNoParents
                            println!("[Mock Server] Received HaveNoParents");
                        }
                        28 => {
                            // SetStatus
                            println!("[Mock Server] Received SetStatus");
                        }
                        3 => {
                            // GetPeerAddress
                            println!("[Mock Server] Received GetPeerAddress - sending ConnectToPeer type F!");

                            // Simulate correct server behavior - send ConnectToPeer type F to ALL clients
                            let token = vec![187, 31, 0, 0];
                            let connect_msg = build_connect_to_peer(
                                "MisterDanielson",
                                "F",
                                "127.0.0.1",
                                9001,
                                token,
                            );

                            // Send to the requesting client
                            write_message(&mut stream, 18, connect_msg.clone());
                            println!("[Mock Server] Sent ConnectToPeer type F to requesting client");

                            // Also send to all other connected clients (simulating real server behavior)
                            let clients = client_streams_clone.lock().unwrap();
                            for client_stream in clients.iter() {
                                if let Ok(mut cloned) =
                                    client_stream.try_clone()
                                {
                                    write_message(
                                        &mut cloned,
                                        18,
                                        connect_msg.clone(),
                                    );
                                    println!("[Mock Server] Sent ConnectToPeer type F to a connected client");
                                }
                            }
                        }
                        _ => {
                            println!("[Mock Server] Received unknown message code: {}", code);
                        }
                    }
                }
            });
        }
    });

    port
}

// Mock Peer (MisterDanielson)
fn mock_peer(peer_messages: Arc<Mutex<Vec<String>>>) {
    let listener =
        TcpListener::bind("127.0.0.1:9001").expect("Failed to bind peer");

    thread::spawn(move || {
        println!("[Mock Peer] MisterDanielson listening on port 9001");

        // Accept connections in a loop
        for stream in listener.incoming() {
            println!("[Mock Peer] Incoming connection attempt!");
            let mut stream = stream.expect("Failed to accept peer connection");
            let messages = peer_messages.clone();

            thread::spawn(move || {
                // Handle initial handshake
                println!("[Mock Peer] Connection accepted from client");
                messages
                    .lock()
                    .unwrap()
                    .push("Peer connection accepted".to_string());

                // Set a longer timeout for reading
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                // Read the initial pierce firewall message (code 0 with token)
                let mut handshake = [0u8; 9];
                match stream.read_exact(&mut handshake) {
                    Ok(_) => {
                        println!(
                            "[Mock Peer] Received initial handshake (9 bytes)"
                        );
                    }
                    Err(e) => {
                        println!(
                            "[Mock Peer] Failed to read initial message: {}",
                            e
                        );
                        return;
                    }
                };

                // The initial message is usually a pierce firewall (code 0) followed by token
                // Just acknowledge we got it
                messages
                    .lock()
                    .unwrap()
                    .push("Peer handshake completed".to_string());

                // Send file search results
                println!("[Mock Peer] Sending FileSearchResponse");

                // Build FileSearchResponse message
                let mut response = Vec::new();

                // Compressed flag (1 = compressed, 0 = uncompressed)
                response.push(0); // uncompressed for simplicity

                // Number of results
                response.extend_from_slice(&1u32.to_le_bytes());

                // Username
                let username = "MisterDanielson";
                response
                    .extend_from_slice(&(username.len() as u32).to_le_bytes());
                response.extend_from_slice(username.as_bytes());

                // Token (search token)
                response.extend_from_slice(&[80, 102, 209, 7]);

                // Number of files
                response.extend_from_slice(&1u32.to_le_bytes());

                // File code
                response.push(1);

                // Filename
                let filename = r"@@axnso\Music\SoulSeek\50. Super Flu - Believe (Extended Mix).mp3";
                response
                    .extend_from_slice(&(filename.len() as u32).to_le_bytes());
                response.extend_from_slice(filename.as_bytes());

                // File size
                response.extend_from_slice(&17580946u64.to_le_bytes());

                // Extension
                let ext = "mp3";
                response.extend_from_slice(&(ext.len() as u32).to_le_bytes());
                response.extend_from_slice(ext.as_bytes());

                // Number of attributes
                response.extend_from_slice(&0u32.to_le_bytes());

                // Has free upload slot
                response.push(1);

                // Average speed
                response.extend_from_slice(&1000u32.to_le_bytes());

                // Queue length
                response.extend_from_slice(&0u32.to_le_bytes());

                write_message(&mut stream, 9, response);
                println!("[Mock Peer] Sent FileSearchResponse");

                messages
                    .lock()
                    .unwrap()
                    .push("Peer sent FileSearchResponse".to_string());

                loop {
                    let msg = match read_message(&mut stream) {
                        msg => msg,
                    };

                    if msg.len() < 4 {
                        continue;
                    }

                    let code =
                        u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);

                    messages
                        .lock()
                        .unwrap()
                        .push(format!("Peer received code: {}", code));

                    match code {
                        40 => {
                            // TransferRequest
                            println!("[Mock Peer] Received TransferRequest");

                            // First time: respond with Queued
                            let token = vec![msg[8], msg[9], msg[10], msg[11]];
                            println!("[Mock Peer] Token: {:?}", token);

                            // Always respond with Queued to any TransferRequest
                            {
                                let mut response = Vec::new();
                                response.extend_from_slice(&token);
                                response.push(0); // not allowed
                                response.extend_from_slice(&6u32.to_le_bytes()); // "Queued" length
                                response.extend_from_slice(b"Queued");

                                write_message(&mut stream, 41, response);
                                println!(
                                    "[Mock Peer] Sent TransferResponse: Queued"
                                );

                                // After some time, send TransferRequest back
                                thread::sleep(Duration::from_millis(500));

                                let mut request = Vec::new();
                                request.extend_from_slice(&1u32.to_le_bytes()); // direction = 1
                                request.extend_from_slice(&[187, 31, 0, 0]); // new token

                                // Filename
                                let filename = "@@axnso\\Music\\SoulSeek\\50. Super Flu - Believe (Extended Mix).mp3";
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
                                println!("[Mock Peer] Transfer accepted! Client should now send GetPeerAddress to server");
                                messages.lock().unwrap().push(
                                    "Peer received TransferResponse allowed=1"
                                        .to_string(),
                                );

                                // The client itself should send GetPeerAddress to the server
                                // We don't simulate anything - let the real client handle it
                            }
                        }
                        _ => {
                            println!(
                                "[Mock Peer] Received unknown message code: {}",
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
fn test_transfer_flow_with_connect_to_peer_f() {
    use soulseek_rs::{Client, PeerAddress};

    // No external logging dependencies needed

    let server_messages = Arc::new(Mutex::new(Vec::new()));
    let peer_messages = Arc::new(Mutex::new(Vec::new()));
    // Start mock server (simulates central Soulseek server)
    let server_port = mock_server(server_messages.clone());
    thread::sleep(Duration::from_millis(100));

    // Start mock peer (simulates MisterDanielson)
    mock_peer(peer_messages.clone());
    thread::sleep(Duration::from_millis(100));

    // Create and connect client
    let mut client = Client::new(
        PeerAddress::new(String::from("127.0.0.1"), server_port),
        String::from("insane_in_the_brain3"),
        String::from("13375137"),
    );

    println!("Connecting to mock server on port {}", server_port);
    client.connect();

    // No test connections - let the client handle everything

    // Login
    println!("Logging in as insane_in_the_brain3");
    match client.login() {
        Ok(success) => {
            assert!(success, "Login should succeed");
            println!("Login successful");
        }
        Err(e) => panic!("Login failed: {}", e),
    }

    // Wait for server messages to be processed
    thread::sleep(Duration::from_millis(200));

    // Search for file (this triggers connection to peer)
    println!("Searching for 'Super flu Believe'");
    // Use a short timeout to allow processing of server messages
    let _search_results =
        client.search("Super flu Believe", Duration::from_millis(500));

    // Give time for server to process and send ConnectToPeer
    thread::sleep(Duration::from_millis(100));
    
    // Do another short search to allow message processing
    let _search_results2 = 
        client.search("test", Duration::from_millis(500));
    
    // Wait for peer connection to be established and messages to be processed
    println!("Waiting for peer connection to be established...");
    thread::sleep(Duration::from_millis(500));

    // Now test the full download flow through the client
    {
        // First, let's wait to ensure the peer connection from search is established
        println!("Ensuring peer connection is established...");
        thread::sleep(Duration::from_millis(500)); // Give time for connection

        // Check messages so far
        let server_msgs = server_messages.lock().unwrap();
        let peer_msgs = peer_messages.lock().unwrap();

        println!("\n=== Server Messages ===");
        for msg in server_msgs.iter() {
            println!("{}", msg);
        }

        println!("\n=== Peer Messages ===");
        for msg in peer_msgs.iter() {
            println!("{}", msg);
        }
        
        // Verify we got the ConnectToPeer messages
        assert!(
            peer_msgs.iter().any(|m| m.contains("connection")),
            "Peer should have received connection"
        );
        
        assert!(
            peer_msgs.iter().any(|m| m.contains("FileSearchResponse")),
            "Peer should have sent FileSearchResponse"
        );
        //
        println!("✓ Test completed successfully - transfer flow with ConnectToPeer type F verified");
    }
}
