use super::*;
use anyhow::{Context, ensure};
use serde_json::json;
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
};

pub fn serve_stdio(root: PathBuf, caller: ThreadId, expected_identity: Option<&str>) -> Result<()> {
    let resident = Arc::new(crate::resident::ResidentStore::open(root.clone())?);
    use relay_core::threads::ThreadService;
    ensure!(resident.thread(caller).is_some(), "Unknown tool scope");
    let provider = super::open(&root)?;
    if let Some(expected) = expected_identity {
        ensure!(
            expected == provider.identity(),
            "Memory provider changed; restart Relay before opening another MCP session"
        );
    }
    serve(
        &resident,
        &*provider,
        caller,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

pub(super) fn serve(
    resident: &crate::resident::ResidentStore,
    provider: &dyn MemoryProvider,
    caller: ThreadId,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let mut tools = crate::resident::task_tools(caller);
    tools.extend(provider.tools(caller));
    loop {
        let mut line = String::new();
        let n = (&mut input).take(262145).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        ensure!(
            n <= 262144 && line.ends_with('\n'),
            "MCP message exceeds size limit"
        );
        let request: Value = serde_json::from_str(&line)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),"capabilities":{"tools":{}},"serverInfo":{"name":"relay","version":"2.0.0"}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":tools})),
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                let args = &request["params"]["arguments"];
                let value = (|| -> Result<Value> {
                    ensure!(
                        tools.iter().any(|t| t["name"] == name),
                        "Tool unavailable in this session"
                    );
                    if name.starts_with("memory_") {
                        provider.call(caller, name, args)
                    } else {
                        resident.call_task(caller, name, args)
                    }
                })();
                Ok(match value {
                    Ok(v) => json!({"content":[{"type":"text","text":v.to_string()}]}),
                    Err(e) => {
                        json!({"isError":true,"content":[{"type":"text","text":e.to_string()}]})
                    }
                })
            }
            _ => Err(json!({"code":-32601,"message":"Method not found"})),
        };
        let message = match result {
            Ok(r) => json!({"jsonrpc":"2.0","id":id,"result":r}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,"error":e}),
        };
        writeln!(output, "{message}").context("Writing MCP response")?;
        output.flush()?;
    }
}
