//! Server — Axum HTTP 应用构建
//!
//! 提供 `build_app()` 供 wem-kernel 和 wem-cli 共用。

use std::sync::Arc;

use axum::{Router, routing::{get, post}, extract::DefaultBodyLimit};
use axum::http::header::HeaderValue;
use tower_http::cors::{CorsLayer, Any};

use crate::agent::handler as agent_handler;
use crate::agent::handler::AgentState;
use crate::block_system::handler;
use crate::block_system::handler::asset as asset_handler;
use crate::block_system::handler::asset::AssetState;
use crate::repo::Db;

pub fn build_app(
    db: Db,
    agent_state: Arc<AgentState>,
    asset_data_root: String,
    cors_origin: &str,
) -> Router {
    let block_routes = Router::new()
        .route("/api/v1/health", get(handler::health))
        .route("/api/v1/documents", post(handler::create_document))
        .route("/api/v1/documents/list", post(handler::list_documents))
        .route("/api/v1/documents/get", post(handler::get_document))
        .route("/api/v1/documents/breadcrumb", post(handler::get_breadcrumb))
        .route("/api/v1/documents/children", post(handler::get_document_children))
        .route("/api/v1/documents/delete", post(handler::delete_document))
        .route("/api/v1/documents/export", post(handler::export_text))
        .route("/api/v1/documents/import", post(handler::import_text))
        .route("/api/v1/documents/move", post(handler::move_document_tree))
        .route("/api/v1/documents/history", post(handler::get_block_history))
        .route("/api/v1/documents/undo", post(handler::undo))
        .route("/api/v1/documents/redo", post(handler::redo))
        .route("/api/v1/documents/{id}/events", get(handler::document_events))
        .route("/api/v1/blocks/create", post(handler::create_block))
        .route("/api/v1/blocks/get", post(handler::get_block))
        .route("/api/v1/blocks/update", post(handler::update_block))
        .route("/api/v1/blocks/delete", post(handler::delete_block))
        .route("/api/v1/blocks/delete-tree", post(handler::delete_tree))
        .route("/api/v1/blocks/move", post(handler::move_block))
        .route("/api/v1/blocks/move-tree", post(handler::move_heading_tree))
        .route("/api/v1/blocks/export", post(handler::export_block))
        .route("/api/v1/blocks/restore", post(handler::restore_block))
        .route("/api/v1/blocks/split", post(handler::split_block))
        .route("/api/v1/blocks/merge", post(handler::merge_block))
        .route("/api/v1/blocks/batch", post(handler::batch_blocks))
        .with_state(db);

    let agent_routes = Router::new()
        .route("/api/v1/agent/health", get(agent_handler::health))
        .route("/api/v1/agent/sessions", post(agent_handler::create_session))
        .route("/api/v1/agent/sessions/list", post(agent_handler::list_sessions))
        .route("/api/v1/agent/sessions/{id}", post(agent_handler::destroy_session))
        .route("/api/v1/agent/sessions/{id}/chat", post(agent_handler::chat))
        .route("/api/v1/agent/sessions/{id}/events", get(agent_handler::events))
        .route("/api/v1/agent/sessions/{id}/abort", post(agent_handler::abort_session))
        .route("/api/v1/agent/sessions/{id}/permission", post(agent_handler::resolve_permission))
        .with_state(agent_state);

    let asset_state = AssetState {
        data_root: asset_data_root,
    };
    let asset_routes = asset_handler::asset_router(asset_state);

    let cors_layer = CorsLayer::new()
        .allow_origin(if cors_origin.is_empty() {
            tower_http::cors::AllowOrigin::any()
        } else {
            tower_http::cors::AllowOrigin::exact(
                cors_origin.parse::<HeaderValue>().unwrap()
            )
        })
        .allow_methods(Any)
        .allow_headers(Any);

    block_routes
        .merge(agent_routes)
        .merge(asset_routes)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(cors_layer)
}
