//! Interests and the queries built on them: recommendations (codes 54, 56,
//! 111), similar users (110, 112) and another user's interests (57).
//!
//! Ratings are signed on the wire: the server sends a descending list of
//! things it recommends and an ascending list of things it recommends
//! against, and the latter carry negative ratings.

use std::sync::mpsc::Sender;

use crate::{
    actor::server_actor::ServerMessage,
    message::{Message, MessageHandler},
    types::{Recommendation, SimilarUser, UserInterests},
};

/// Read a `[count][item][rating]...` vector, stopping if the payload runs out
/// before the count does.
fn read_recommendations(message: &mut Message) -> Vec<Recommendation> {
    let count = message.read_int32();
    let mut items = Vec::new();
    for _ in 0..count {
        let item = message.read_string();
        if item.is_empty() {
            break;
        }
        #[allow(clippy::cast_possible_wrap)]
        let rating = message.read_int32() as i32;
        items.push(Recommendation { item, rating });
    }
    items
}

/// Read a `[count][string]...` vector, stopping at the end of the payload.
fn read_strings(message: &mut Message) -> Vec<String> {
    let count = message.read_int32();
    let mut items = Vec::new();
    for _ in 0..count {
        let item = message.read_string();
        if item.is_empty() {
            break;
        }
        items.push(item);
    }
    items
}

/// `GetRecommendations` (code 54): recommendations drawn from our own
/// interests.
pub struct RecommendationsHandler;

impl MessageHandler<ServerMessage> for RecommendationsHandler {
    fn get_code(&self) -> u32 {
        54
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let recommended = read_recommendations(message);
        let unrecommended = read_recommendations(message);
        let _ = sender.send(ServerMessage::RecommendationsReceived {
            recommended,
            unrecommended,
        });
    }
}

/// `GlobalRecommendations` (code 56): the same shape, server-wide.
pub struct GlobalRecommendationsHandler;

impl MessageHandler<ServerMessage> for GlobalRecommendationsHandler {
    fn get_code(&self) -> u32 {
        56
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let recommended = read_recommendations(message);
        let unrecommended = read_recommendations(message);
        let _ = sender.send(ServerMessage::GlobalRecommendationsReceived {
            recommended,
            unrecommended,
        });
    }
}

/// `ItemRecommendations` (code 111): what else people who like one item like.
pub struct ItemRecommendationsHandler;

impl MessageHandler<ServerMessage> for ItemRecommendationsHandler {
    fn get_code(&self) -> u32 {
        111
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let item = message.read_string();
        let recommendations = read_recommendations(message);
        let _ = sender.send(ServerMessage::ItemRecommendationsReceived {
            item,
            recommendations,
        });
    }
}

/// `SimilarUsers` (code 110): users whose interests overlap ours.
pub struct SimilarUsersHandler;

impl MessageHandler<ServerMessage> for SimilarUsersHandler {
    fn get_code(&self) -> u32 {
        110
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let count = message.read_int32();
        let mut users = Vec::new();
        for _ in 0..count {
            let username = message.read_string();
            if username.is_empty() {
                break;
            }
            let weight = message.read_int32();
            users.push(SimilarUser { username, weight });
        }
        let _ = sender.send(ServerMessage::SimilarUsersReceived { users });
    }
}

/// `ItemSimilarUsers` (code 112): who likes one item. The server sends only
/// names here, so every weight reads as zero.
pub struct ItemSimilarUsersHandler;

impl MessageHandler<ServerMessage> for ItemSimilarUsersHandler {
    fn get_code(&self) -> u32 {
        112
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let item = message.read_string();
        let usernames = read_strings(message);
        let _ = sender
            .send(ServerMessage::ItemSimilarUsersReceived { item, usernames });
    }
}

/// `UserInterests` (code 57): what one user likes and hates.
pub struct UserInterestsHandler;

impl MessageHandler<ServerMessage> for UserInterestsHandler {
    fn get_code(&self) -> u32 {
        57
    }

    fn handle(&self, message: &mut Message, sender: Sender<ServerMessage>) {
        let username = message.read_string();
        let likes = read_strings(message);
        let hates = read_strings(message);
        let _ =
            sender.send(ServerMessage::UserInterestsReceived(UserInterests {
                username,
                likes,
                hates,
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::framed;

    #[test]
    fn recommendations_split_into_recommended_and_not() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(1);
            m.write_string("jazz");
            m.write_int32(3);
            m.write_int32(1);
            m.write_string("polka");
            #[allow(clippy::cast_sign_loss)]
            m.write_int32(-2i32 as u32);
        });

        RecommendationsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::RecommendationsReceived {
                recommended,
                unrecommended,
            }) => {
                assert_eq!(
                    recommended,
                    vec![Recommendation {
                        item: "jazz".into(),
                        rating: 3
                    }]
                );
                assert_eq!(
                    unrecommended,
                    vec![Recommendation {
                        item: "polka".into(),
                        rating: -2
                    }]
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn global_recommendations_parse_the_same_shape() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(1);
            m.write_string("dub");
            m.write_int32(9);
            m.write_int32(0);
        });

        GlobalRecommendationsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::GlobalRecommendationsReceived {
                recommended,
                unrecommended,
            }) => {
                assert_eq!(recommended.len(), 1);
                assert_eq!(recommended[0].item, "dub");
                assert!(unrecommended.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn similar_users_carry_their_weight() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_int32(2);
            m.write_string("alice");
            m.write_int32(4);
            m.write_string("bob");
            m.write_int32(1);
        });

        SimilarUsersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::SimilarUsersReceived { users }) => {
                assert_eq!(
                    users,
                    vec![
                        SimilarUser {
                            username: "alice".into(),
                            weight: 4
                        },
                        SimilarUser {
                            username: "bob".into(),
                            weight: 1
                        },
                    ]
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn item_similar_users_report_the_item_they_answer_for() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_int32(1);
            m.write_string("alice");
        });

        ItemSimilarUsersHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::ItemSimilarUsersReceived { item, usernames }) => {
                assert_eq!(item, "jazz");
                assert_eq!(usernames, vec!["alice".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn item_recommendations_report_their_item() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("jazz");
            m.write_int32(1);
            m.write_string("blues");
            m.write_int32(2);
        });

        ItemRecommendationsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::ItemRecommendationsReceived {
                item,
                recommendations,
            }) => {
                assert_eq!(item, "jazz");
                assert_eq!(recommendations[0].item, "blues");
                assert_eq!(recommendations[0].rating, 2);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_users_likes_and_hates_arrive_as_two_lists() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("alice");
            m.write_int32(2);
            m.write_string("jazz");
            m.write_string("dub");
            m.write_int32(1);
            m.write_string("polka");
        });

        UserInterestsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::UserInterestsReceived(interests)) => {
                assert_eq!(interests.username, "alice");
                assert_eq!(interests.likes, ["jazz", "dub"]);
                assert_eq!(interests.hates, ["polka"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_user_with_no_interests_parses_to_empty_lists() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut message = framed(|m| {
            m.write_string("ghost");
            m.write_int32(0);
            m.write_int32(0);
        });

        UserInterestsHandler.handle(&mut message, tx);
        match rx.try_recv() {
            Ok(ServerMessage::UserInterestsReceived(interests)) => {
                assert!(interests.likes.is_empty());
                assert!(interests.hates.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
