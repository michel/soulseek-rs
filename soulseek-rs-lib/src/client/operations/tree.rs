//! Dispatch arms for our place in the distributed search tree.
//!
//! Lifted out of the dispatch loop whole, so the bodies read exactly as they
//! did inside it.

use super::{Arc, Client, ClientContext, ClientOperation, RwLock, RwLockExt};

impl Client {
    pub(super) fn on_tree(
        operation: ClientOperation,
        client_context: &Arc<RwLock<ClientContext>>,
    ) {
        match operation {
            ClientOperation::ParentSearch {
                parent,
                link,
                username,
                token,
                query,
            } => {
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                if let Some(branch) = ctx.leaf.search_from(&parent, link) {
                    crate::client::distributed::announce_move(
                        &mut ctx, &branch, true,
                    );
                }
                if ctx.leaf.is_parent(&parent, link) {
                    // The tree's stream is not ours to throttle: every
                    // search goes down to our children. The budget
                    // below bounds only the share scan we do for our
                    // own reply.
                    ctx.children.broadcast_search(&username, token, &query);
                    if let Some(ops) = &ctx.operations {
                        let _ = ops.send(ClientOperation::IncomingSearch {
                            username,
                            token,
                            query,
                            from_parent: true,
                        });
                    }
                }
            }
            ClientOperation::ParentBranchLevel {
                parent,
                link,
                level,
            } => {
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                if let Some(branch) =
                    ctx.leaf.branch_level(&parent, link, level)
                {
                    crate::client::distributed::announce_move(
                        &mut ctx, &branch, true,
                    );
                }
            }
            ClientOperation::ParentBranchRoot { parent, link, root } => {
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                if let Some(branch) = ctx.leaf.branch_root(&parent, link, &root)
                {
                    crate::client::distributed::announce_move(
                        &mut ctx, &branch, true,
                    );
                }
            }
            ClientOperation::ParentClosed { parent, link } => {
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                if ctx.leaf.closed(&parent, link) {
                    let branch = ctx.leaf.branch();
                    crate::client::distributed::announce_move(
                        &mut ctx, &branch, false,
                    );
                }
            }
            // Also what a fresh login means: whatever tree we hung
            // from belongs to the old session.
            ClientOperation::ResetDistributed => {
                let Ok(mut ctx) = client_context.write_safe() else {
                    return;
                };
                ctx.leaf.reset();
                // A fresh session feeds us nothing until the tree (or
                // the server) starts again, so the children we were
                // carrying are let go rather than left starving.
                ctx.set_fed_by(false);
                let branch = ctx.leaf.branch();
                crate::client::distributed::announce_move(
                    &mut ctx, &branch, false,
                );
            }
            // The dispatch loop routes only the variants above to here.
            _ => {}
        }
    }
}
