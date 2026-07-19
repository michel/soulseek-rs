use std::io;
use std::net::{SocketAddr, TcpStream as StdTcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mio::event::Event;
use mio::net::{TcpListener, TcpStream};
use mio::{Interest, Registry, Token};

use crate::actor::{Actor, ActorHandle, ActorSystem};
use crate::client::ClientOperation;
use crate::message::{Message, MessageReader};
use crate::peer::{ConnectionType, Peer};
use crate::{debug, error, info, trace};

const PEER_INIT_MESSAGE_CODE: u8 = 1;
const PIERCE_FIREWALL_MESSAGE_CODE: u8 = 0;
const PEER_INIT_TIMEOUT: Duration = Duration::from_secs(10);

struct PeerInitData {
    username: String,
    connection_type: ConnectionType,
    token: u32,
}

fn parse_peer_init_message(mut message: Message) -> Option<PeerInitData> {
    message.set_pointer(4);
    let message_code = message.read_int8();

    if message_code != PEER_INIT_MESSAGE_CODE {
        return None;
    }

    let username = message.read_string();
    let connection_type = message.read_string().parse().ok()?;
    Some(PeerInitData {
        username,
        connection_type,
        token: message.read_int32(),
    })
}

pub struct ListenActor {
    listener: TcpListener,
    actor_system: Arc<ActorSystem>,
    client_sender: ActorHandle<ClientOperation>,
}

impl ListenActor {
    pub fn bind(
        port: u16,
        actor_system: Arc<ActorSystem>,
        client_sender: ActorHandle<ClientOperation>,
    ) -> io::Result<Self> {
        let address = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = TcpListener::bind(address)?;
        info!("[listener] starting listener on port {port}");

        Ok(Self {
            listener,
            actor_system,
            client_sender,
        })
    }

    fn accept_connections(&self) {
        loop {
            match self.listener.accept() {
                Ok((stream, peer_addr)) => {
                    let actor = IncomingConnectionActor::new(
                        stream,
                        peer_addr,
                        self.client_sender.clone(),
                    );
                    self.actor_system.spawn(actor);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    error!("[listener] Failed to accept connection: {}", e);
                    break;
                }
            }
        }
    }
}

impl Actor for ListenActor {
    type Message = ();

    fn handle(&mut self, _msg: Self::Message) {}

    fn handle_io_event(&mut self, _registry: &Registry, event: &Event) {
        if event.is_readable() {
            self.accept_connections();
        }
    }

    fn register_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        registry.register(&mut self.listener, token, Interest::READABLE)?;
        Ok(true)
    }

    fn reregister_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        registry.reregister(&mut self.listener, token, Interest::READABLE)?;
        Ok(true)
    }

    fn deregister_io(&mut self, registry: &Registry) -> io::Result<()> {
        registry.deregister(&mut self.listener)
    }
}

struct IncomingConnectionActor {
    stream: Option<TcpStream>,
    peer_addr: SocketAddr,
    reader: MessageReader,
    client_sender: ActorHandle<ClientOperation>,
    accepted_at: Instant,
    done: bool,
}

impl IncomingConnectionActor {
    fn new(
        stream: TcpStream,
        peer_addr: SocketAddr,
        client_sender: ActorHandle<ClientOperation>,
    ) -> Self {
        Self {
            stream: Some(stream),
            peer_addr,
            reader: MessageReader::new(),
            client_sender,
            accepted_at: Instant::now(),
            done: false,
        }
    }

    fn process_readable(&mut self, registry: &Registry) {
        loop {
            let Some(stream) = self.stream.as_mut() else {
                self.done = true;
                return;
            };

            match self.reader.read_from_socket(stream) {
                Ok(()) => {
                    if self.try_handoff_connection(registry) {
                        return;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    error!(
                        "[listener:{}] Failed to read peer init message: {}",
                        self.peer_addr, e
                    );
                    self.done = true;
                    return;
                }
            }
        }
    }

    fn try_handoff_connection(&mut self, registry: &Registry) -> bool {
        let message = match self.reader.extract_message() {
            Ok(Some(message)) => message,
            Ok(None) => return false,
            Err(e) => {
                error!(
                    "[listener:{}] Invalid peer init frame: {}",
                    self.peer_addr, e
                );
                self.done = true;
                return true;
            }
        };

        if message.get_message_code() == PIERCE_FIREWALL_MESSAGE_CODE {
            self.handoff_pierce_firewall(registry, message);
            return true;
        }

        let Some(init_data) = parse_peer_init_message(message) else {
            error!(
                "[listener:{}] Invalid or unknown peer init message",
                self.peer_addr
            );
            self.done = true;
            return true;
        };

        debug!(
            "[listener:{}] peerInit (0) username: {} connection_type: {} token: {}",
            self.peer_addr,
            init_data.username,
            init_data.connection_type,
            init_data.token
        );

        let peer_ip = self.peer_addr.ip().to_string();
        let peer_port = self.peer_addr.port();
        let peer = Peer::new(
            format!("{}:direct", init_data.username),
            init_data.connection_type.clone(),
            peer_ip.clone(),
            peer_port.into(),
            None,
            0,
            0,
            0,
        );

        let Some(stream) = self.stream.as_mut() else {
            self.done = true;
            return true;
        };
        if let Err(e) = registry.deregister(stream) {
            error!(
                "[listener:{}] failed to deregister peer-init stream before handoff: {}",
                self.peer_addr, e
            );
            self.done = true;
            return true;
        }

        let Some(stream) = self.stream.take() else {
            self.done = true;
            return true;
        };
        let stream: StdTcpStream = stream.into();

        match init_data.connection_type {
            ConnectionType::P => {
                if let Err(e) =
                    self.client_sender.send(ClientOperation::PeerConnection {
                        peer,
                        stream,
                        reader: std::mem::take(&mut self.reader),
                    })
                {
                    error!(
                        "[listener] failed to hand peer connection to client: {}",
                        e
                    );
                }
            }
            ConnectionType::F => {
                trace!(
                    "[listener:{}] handing file connection to client",
                    self.peer_addr
                );
                if let Err(e) = stream.set_nonblocking(false) {
                    error!(
                        "[listener:{}] failed to make file stream blocking: {}",
                        self.peer_addr, e
                    );
                }
                if let Err(e) = self.client_sender.send(
                    ClientOperation::IncomingFileConnection {
                        peer,
                        stream,
                        reader: std::mem::take(&mut self.reader),
                        token: init_data.token,
                        peer_ip,
                        peer_port,
                    },
                ) {
                    error!(
                        "[listener] failed to hand file connection to client: {}",
                        e
                    );
                }
            }
            ConnectionType::D => {
                debug!(
                    "[listener:{}] connection type is D, not supported yet, closing connection",
                    self.peer_addr
                );
            }
        }

        self.done = true;
        true
    }

    fn handoff_pierce_firewall(
        &mut self,
        registry: &Registry,
        mut message: Message,
    ) {
        message.set_pointer(5);
        let token = message.read_int32();
        let peer_ip = self.peer_addr.ip().to_string();
        let peer_port = self.peer_addr.port();

        let Some(stream) = self.stream.as_mut() else {
            self.done = true;
            return;
        };
        if let Err(e) = registry.deregister(stream) {
            error!(
                "[listener:{}] failed to deregister pierced stream before handoff: {}",
                self.peer_addr, e
            );
            self.done = true;
            return;
        }

        let Some(stream) = self.stream.take() else {
            self.done = true;
            return;
        };
        let stream: StdTcpStream = stream.into();

        if let Err(e) =
            self.client_sender
                .send(ClientOperation::PierceFirewallConnection {
                    token,
                    stream,
                    reader: std::mem::take(&mut self.reader),
                    peer_ip,
                    peer_port,
                })
        {
            error!(
                "[listener] failed to hand pierced peer connection to client: {}",
                e
            );
        }

        self.done = true;
    }
}

impl Actor for IncomingConnectionActor {
    type Message = ();

    fn handle(&mut self, _msg: Self::Message) {}

    fn should_stop(&self) -> bool {
        self.done
    }

    fn tick(&mut self) {
        if self.accepted_at.elapsed() >= PEER_INIT_TIMEOUT {
            error!(
                "[listener:{}] timed out waiting for peer init",
                self.peer_addr
            );
            self.done = true;
        }
    }

    fn handle_io_event(&mut self, registry: &Registry, event: &Event) {
        if event.is_readable() {
            self.process_readable(registry);
        }
    }

    fn register_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };
        registry.register(stream, token, Interest::READABLE)?;
        Ok(true)
    }

    fn reregister_io(
        &mut self,
        registry: &Registry,
        token: Token,
    ) -> io::Result<bool> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };
        registry.reregister(stream, token, Interest::READABLE)?;
        Ok(true)
    }

    fn deregister_io(&mut self, registry: &Registry) -> io::Result<()> {
        if let Some(stream) = self.stream.as_mut() {
            registry.deregister(stream)?;
        }
        Ok(())
    }

    fn tick_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
}
