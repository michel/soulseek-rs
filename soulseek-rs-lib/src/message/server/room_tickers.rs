use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
    types::RoomTicker,
};

/// `RoomTickers` (code 113): the whole ticker board of a room, sent when we
/// join it.
pub struct RoomTickersHandler;

impl MessageHandler<ServerMessage> for RoomTickersHandler {
    fn get_code(&self) -> u32 {
        113
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let count = message.read_int32();
        // A truncated or hostile count must not preallocate: grow as entries
        // actually parse, and stop at the first one that runs off the end.
        let mut tickers = Vec::new();
        for _ in 0..count {
            let username = message.read_string();
            let ticker = message.read_string();
            if username.is_empty() {
                break;
            }
            tickers.push(RoomTicker { username, ticker });
        }
        let _ = sender.send(ServerMessage::RoomTickers { room, tickers });
    }
}

/// `RoomTickerAdded` (code 114): one user set or replaced their ticker.
pub struct RoomTickerAddedHandler;

impl MessageHandler<ServerMessage> for RoomTickerAddedHandler {
    fn get_code(&self) -> u32 {
        114
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let ticker = message.read_string();
        let _ = sender.send(ServerMessage::RoomTickerAdded {
            room,
            username,
            ticker,
        });
    }
}

/// `RoomTickerRemoved` (code 115): one user cleared their ticker.
pub struct RoomTickerRemovedHandler;

impl MessageHandler<ServerMessage> for RoomTickerRemovedHandler {
    fn get_code(&self) -> u32 {
        115
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let room = message.read_string();
        let username = message.read_string();
        let _ =
            sender.send(ServerMessage::RoomTickerRemoved { room, username });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn a_ticker_board_carries_every_entry_in_order() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_int32(2);
            m.write_string("alice");
            m.write_string("hello there");
            m.write_string("bob");
            m.write_string("back later");
        });

        RoomTickersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomTickers { room, tickers }) => {
                assert_eq!(room, "jazz");
                assert_eq!(
                    tickers,
                    vec![
                        RoomTicker {
                            username: "alice".into(),
                            ticker: "hello there".into()
                        },
                        RoomTicker {
                            username: "bob".into(),
                            ticker: "back later".into()
                        },
                    ]
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_count_larger_than_the_payload_stops_at_the_data() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_int32(1_000_000);
            m.write_string("alice");
            m.write_string("hi");
        });

        RoomTickersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomTickers { tickers, .. }) => {
                assert_eq!(tickers.len(), 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn an_added_ticker_reports_its_room_user_and_text() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_string("alice");
            m.write_string("new ticker");
        });

        RoomTickerAddedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomTickerAdded {
                room,
                username,
                ticker,
            }) => {
                assert_eq!(
                    (room.as_str(), username.as_str(), ticker.as_str()),
                    ("jazz", "alice", "new ticker")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_removed_ticker_reports_its_room_and_user() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_string("alice");
        });

        RoomTickerRemovedHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RoomTickerRemoved { room, username }) => {
                assert_eq!(
                    (room.as_str(), username.as_str()),
                    ("jazz", "alice")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
