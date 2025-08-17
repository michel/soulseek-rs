use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

// Helper to write a message
fn write_message(stream: &mut TcpStream, code: u32, data: Vec<u8>) {
    let mut message = Vec::new();
    message.extend_from_slice(&code.to_le_bytes());
    message.extend_from_slice(&data);
    
    let size = (message.len() as u32).to_le_bytes();
    stream.write_all(&size).expect("Failed to write size");
    stream.write_all(&message).expect("Failed to write message");
    stream.flush().expect("Failed to flush");
}

// Helper to read a message
fn read_message(stream: &mut TcpStream) -> Vec<u8> {
    let mut size_buf = [0u8; 4];
    stream.read_exact(&mut size_buf).expect("Failed to read size");
    let size = u32::from_le_bytes(size_buf);
    
    let mut message = vec![0u8; size as usize];
    stream.read_exact(&mut message).expect("Failed to read message");
    message
}

// Minimal mock server that just handles login and sends ConnectToPeer
fn mock_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
    let port = listener.local_addr().unwrap().port();
    
    thread::spawn(move || {
        println!("[Mock Server] Listening on port {}", port);
        
        for stream in listener.incoming() {
            let mut stream = stream.expect("Failed to accept");
            println!("[Mock Server] Client connected");
            
            thread::spawn(move || {
                loop {
                    let msg = read_message(&mut stream);
                    if msg.len() < 4 { continue; }
                    
                    let code = u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);
                    println!("[Mock Server] Received message code: {}", code);
                    
                    match code {
                        1 => {
                            // Login - respond with success
                            println!("[Mock Server] Login request received");
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
                            println!("[Mock Server] SharedFoldersFiles received");
                        }
                        71 => {
                            // HaveNoParents - ignore
                            println!("[Mock Server] HaveNoParents received");
                        }
                        28 => {
                            // SetStatus - ignore
                            println!("[Mock Server] SetStatus received");
                        }
                        26 => {
                            // FileSearch - send ConnectToPeer
                            println!("[Mock Server] FileSearch received");
                            
                            // Send immediately
                            
                            // Build ConnectToPeer message
                            let mut connect_msg = Vec::new();
                            
                            // Username
                            let username = "MisterDanielson";
                            connect_msg.extend_from_slice(&(username.len() as u32).to_le_bytes());
                            connect_msg.extend_from_slice(username.as_bytes());
                            
                            // Connection type
                            let conn_type = "P";
                            connect_msg.extend_from_slice(&(conn_type.len() as u32).to_le_bytes());
                            connect_msg.extend_from_slice(conn_type.as_bytes());
                            
                            // IP (127.0.0.1 reversed)
                            connect_msg.extend_from_slice(&[1, 0, 0, 127]);
                            
                            // Port
                            connect_msg.extend_from_slice(&9001u32.to_le_bytes());
                            
                            // Token
                            connect_msg.extend_from_slice(&[1, 2, 3, 4]);
                            
                            // Additional fields
                            connect_msg.push(0); // privileged
                            connect_msg.push(0); // unknown
                            connect_msg.push(0); // obfuscated_port
                            
                            write_message(&mut stream, 18, connect_msg);
                            println!("[Mock Server] Sent ConnectToPeer type P for MisterDanielson");
                        }
                        3 => {
                            // GetPeerAddress - send ConnectToPeer type F
                            println!("[Mock Server] GetPeerAddress received - sending ConnectToPeer type F");
                            
                            // Build ConnectToPeer type F message
                            let mut connect_msg = Vec::new();
                            
                            // Username
                            let username = "MisterDanielson";
                            connect_msg.extend_from_slice(&(username.len() as u32).to_le_bytes());
                            connect_msg.extend_from_slice(username.as_bytes());
                            
                            // Connection type F for file transfer
                            let conn_type = "F";
                            connect_msg.extend_from_slice(&(conn_type.len() as u32).to_le_bytes());
                            connect_msg.extend_from_slice(conn_type.as_bytes());
                            
                            // IP (127.0.0.1 reversed)
                            connect_msg.extend_from_slice(&[1, 0, 0, 127]);
                            
                            // Port
                            connect_msg.extend_from_slice(&9001u32.to_le_bytes());
                            
                            // Token (peer's token)
                            connect_msg.extend_from_slice(&[187, 31, 0, 0]);
                            
                            // Additional fields
                            connect_msg.push(0); // privileged
                            connect_msg.push(0); // unknown
                            connect_msg.push(0); // obfuscated_port
                            
                            write_message(&mut stream, 18, connect_msg);
                            println!("[Mock Server] Sent ConnectToPeer type F for file transfer");
                        }
                        _ => {
                            println!("[Mock Server] Unknown message code: {}", code);
                        }
                    }
                }
            });
        }
    });
    
    port
}

// Minimal mock peer that accepts connections and handles transfers
fn mock_peer() {
    let listener = TcpListener::bind("127.0.0.1:9001").expect("Failed to bind peer");
    
    thread::spawn(move || {
        println!("[Mock Peer] MisterDanielson listening on port 9001");
        
        for stream in listener.incoming() {
            println!("[Mock Peer] Connection received!");
            let mut stream = stream.expect("Failed to accept peer connection");
            
            thread::spawn(move || {
                // Read initial handshake (9 bytes)
                let mut handshake = [0u8; 9];
                match stream.read_exact(&mut handshake) {
                    Ok(_) => println!("[Mock Peer] Received handshake"),
                    Err(e) => {
                        println!("[Mock Peer] Failed to read handshake: {}", e);
                        return;
                    }
                }
                
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
                    
                    if msg.len() < 4 { continue; }
                    
                    let code = u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]);
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
                            println!("[Mock Peer] Sent TransferResponse: Queued");
                            
                            // Send our own TransferRequest immediately
                            thread::sleep(Duration::from_millis(50));
                            
                            let mut request = Vec::new();
                            request.extend_from_slice(&1u32.to_le_bytes()); // direction = 1
                            request.extend_from_slice(&[187, 31, 0, 0]); // peer's token
                            
                            // Filename
                            let filename = r"@@axnso\Music\SoulSeek\50. Super Flu - Believe (Extended Mix).mp3";
                            request.extend_from_slice(&(filename.len() as u32).to_le_bytes());
                            request.extend_from_slice(filename.as_bytes());
                            
                            // File size
                            request.extend_from_slice(&17580946u64.to_le_bytes());
                            
                            write_message(&mut stream, 40, request);
                            println!("[Mock Peer] Sent TransferRequest with token [187, 31, 0, 0]");
                        }
                        41 => {
                            // TransferResponse
                            println!("[Mock Peer] Received TransferResponse");
                            let token = vec![msg[4], msg[5], msg[6], msg[7]];
                            let allowed = msg[8];
                            println!("[Mock Peer] Token: {:?}, Allowed: {}", token, allowed);
                            
                            if allowed == 1 && token == vec![187, 31, 0, 0] {
                                println!("[Mock Peer] Transfer accepted! Client should send GetPeerAddress to server");
                            }
                        }
                        _ => {
                            println!("[Mock Peer] Unknown message code: {}", code);
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
    
    // Start mock server
    let server_port = mock_server();
    thread::sleep(Duration::from_millis(50));
    
    // Start mock peer
    mock_peer();
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
    let filename = r"@@axnso\Music\SoulSeek\50. Super Flu - Believe (Extended Mix).mp3";
    let result = client.download(
        filename.to_string(),
        "MisterDanielson".to_string(),
        17580946,
    );
    println!("Download result: {:?}", result);
    
    // Wait a bit to see the full flow
    thread::sleep(Duration::from_millis(500));
    
    println!("Test completed");
}