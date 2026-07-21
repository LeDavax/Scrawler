//! Scrawler is a portable runtime that lets applications declare their
//! capabilities in a form readable by both humans and agents.
//!
//! The first milestone deliberately focuses on the contract: an XML manifest
//! becomes a typed semantic tree. Future milestones will wire up Lua handlers
//! and expose that tree through MCP.

pub mod bundle;
pub mod config;
pub mod ipc;
pub mod manifest;
pub mod mcp;
pub mod renderer;
pub mod runtime;
pub mod storage;
