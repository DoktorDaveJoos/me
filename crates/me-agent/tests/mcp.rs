use me_agent::{BridgeServer, bridge_call, run_mcp};
use me_core::Vault;
use serde_json::{Value, json};
use std::{
    io::Cursor,
    sync::{Arc, Mutex},
};
#[test]
fn real_socket_and_stdio_obey_grant_revocation_and_tool_allowlist() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("v");
    let mut vault = Vault::create(&root, "synthetic-test-passphrase").unwrap();
    vault.save_note(None, "Steuer-ID", "01234567890").unwrap();
    let scope = vault.shareable_scope().unwrap();
    let session = Arc::new(Mutex::new(Some(vault)));
    let server = BridgeServer::start(&root, session.clone(), scope).unwrap();
    let input=[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"me_facts_get","arguments":{"fields":["person.tax_id"]}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute_sql","arguments":{}}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")+"\n";
    let mut out = Vec::new();
    run_mcp(Cursor::new(input), &mut out, &root).unwrap();
    let out = String::from_utf8(out).unwrap();
    let rows = out
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1]["result"]["tools"].as_array().unwrap().len(), 4);
    assert_eq!(
        rows[2]["result"]["structuredContent"]["facts"][0]["value"],
        "01234567890"
    );
    assert_eq!(rows[3]["error"]["code"], -32602);
    assert!(
        bridge_call(
            &root,
            "me_facts_get",
            json!({"fields":["person.tax_id"],"scope":"all"})
        )
        .get("error")
        .is_some()
    );
    server.revoke();
    let reply = bridge_call(&root, "me_facts_get", json!({"fields":["person.tax_id"]}));
    assert!(!reply.to_string().contains("01234567890"));
    drop(server);
    assert!(!root.join("codex-bridge.json").exists());
}
#[test]
fn no_plaintext_in_descriptor_and_bad_token_cannot_read() {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("v");
    let mut vault = Vault::create(&root, "synthetic-test-passphrase").unwrap();
    vault
        .save_note(None, "Notiz", "PRIVATE_TEST_MARKER")
        .unwrap();
    let server =
        BridgeServer::start(&root, Arc::new(Mutex::new(Some(vault))), Default::default()).unwrap();
    let raw = std::fs::read_to_string(root.join("codex-bridge.json")).unwrap();
    assert!(!raw.contains("PRIVATE_TEST_MARKER"));
    let d: Value = serde_json::from_str(&raw).unwrap();
    let mut socket = UnixStream::connect(d["socket"].as_str().unwrap()).unwrap();
    writeln!(
        socket,
        "{}",
        json!({"token":"wrong","name":"me_documents_search","arguments":{"query":"PRIVATE"}})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line).unwrap();
    assert!(line.contains("access_denied"));
    assert!(!line.contains("PRIVATE_TEST_MARKER"));
    drop(server);
}
#[test]
fn locked_vault_returns_actionable_error_without_opening_database() {
    let t = tempfile::tempdir().unwrap();
    let input = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string() + "\n";
    let mut out = Vec::new();
    run_mcp(Cursor::new(input), &mut out, t.path()).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("Initialize first"));
    assert_eq!(
        bridge_call(t.path(), "me_status", json!({}))["error"],
        "vault_unavailable"
    );
}
