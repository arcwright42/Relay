use super::*;
use std::io::{BufRead, Read, Write};

/// The app's headless MCP entrypoint. Only newline-delimited protocol JSON goes to stdout.
pub fn serve_stdio(root: PathBuf, thread: ThreadId) -> Result<()> {
    let store = NativeStore::open(root)?;
    ensure!(store.thread(thread).is_some(), "Unknown tool scope");
    serve(
        &store,
        thread,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

pub(super) fn serve(
    store: &NativeStore,
    thread: ThreadId,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    loop {
        let mut line = String::new();
        let read = (&mut input).take(262145).read_line(&mut line)?;
        if read == 0 {
            break;
        }
        ensure!(
            read <= 262144 && line.ends_with('\n'),
            "MCP message exceeds size limit"
        );
        let request: Value = serde_json::from_str(&line)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let response = match request["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),"capabilities":{"tools":{}},"serverInfo":{"name":"relay-native","version":"1.0.0"}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":tool_list(thread)})),
            "tools/call" => {
                let result = store.call(
                    thread,
                    request["params"]["name"].as_str().unwrap_or(""),
                    &request["params"]["arguments"],
                );
                Ok(match result {
                    Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
                    Err(e) => {
                        json!({"isError":true,"content":[{"type":"text","text":format!("{e:#}")} ]})
                    }
                })
            }
            _ => Err(json!({"code":-32601,"message":"Method not found"})),
        };
        let message = match response {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
        };
        writeln!(output, "{message}")?;
        output.flush()?;
    }
    Ok(())
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
fn note_schema() -> Value {
    let mut schema = json!({"type":"object","properties":{"title":{"type":"string"},"body":{"type":"string"},"kind":{"type":"string","enum":["fact","decision","preference","working","observation","summary"]},"scope":{"type":["integer","null"]},"status":{"type":"string","enum":["candidate","confirmed"]},"sources":{"type":"array","items":{"type":"object","properties":{"source_id":{"type":"integer"},"revision":{"type":"integer"}},"required":["source_id","revision"]}},"supersedes":{"type":["integer","null"]},"topics":{"type":"array","items":{"type":"string"}}},"required":["title","body"]});
    for name in ["facts", "concepts", "files_read", "files_modified"] {
        schema["properties"][name] =
            json!({"type":"array","items":{"type":"string"},"maxItems":32});
    }
    schema
}
fn summary_schema() -> Value {
    let fields = [
        "request",
        "investigated",
        "learned",
        "completed",
        "next_steps",
        "notes",
    ];
    let properties = fields
        .iter()
        .map(|s| ((*s).to_owned(), json!({"type":"string","maxLength":1800})))
        .collect::<serde_json::Map<_, _>>();
    json!({"type":"object","properties":properties,"required":fields,"additionalProperties":false})
}
fn tool_list(caller: ThreadId) -> Vec<Value> {
    if caller == OBSERVER {
        return vec![tool(
            "memory_commit_job",
            "Atomically acknowledge a leased job. Observation jobs return notes (empty is valid); summary jobs return the six-field session summary. Never substitute observations for a summary checkpoint.",
            json!({"job_id":{"type":"integer"},"attempt":{"type":"integer"},"notes":{"type":"array","items":note_schema()},"summary":summary_schema()}),
            &["job_id", "attempt"],
        )];
    }
    let mut tools = vec![
        tool(
            "memory_search",
            "Find a compact hybrid semantic/keyword index. Returns results plus retrieval mode and any degradation warning. Candidates are unverified. Search first, inspect neighboring evidence with timeline, then batch memory_get only the IDs you need. sources=true searches raw evidence by keywords.",
            json!({"query":{"type":"string"},"sources":{"type":"boolean"},"kind":{"type":"string"},"topic":{"type":"string"},"session":{"type":"string"}}),
            &["query"],
        ),
        tool(
            "memory_get",
            "Read selected memory details and source references, at most 10 IDs. Evidence is paged in groups of 32; use next_evidence_offset to continue.",
            json!({"ids":{"type":"array","items":{"type":"integer"}},"evidence_offset":{"type":"integer","minimum":0}}),
            &["ids"],
        ),
        tool(
            "memory_timeline",
            "Read source evidence and neighboring events by source_id.",
            json!({"source_id":{"type":"integer"}}),
            &["source_id"],
        ),
        tool(
            "memory_write",
            "Save a source-grounded fact, decision, preference or working state. Evidence sources with source_id and revision required. Null scope means global; task agents write only their task. Only the coordinator may confirm or supersede.",
            json!({"note":note_schema()}),
            &["note"],
        ),
        tool(
            "memory_source",
            "Read original evidence in pages of 12000 characters. Follow next_offset until null.",
            json!({"source_id":{"type":"integer"},"offset":{"type":"integer","minimum":0}}),
            &["source_id"],
        ),
        tool(
            "inspect_task",
            "Read task status, handoff and recent archived events. This does not resume execution.",
            json!({"task_id":{"type":"integer"}}),
            &["task_id"],
        ),
        tool(
            "update_task",
            "Record task progress and a handoff. A finished execution should be review; only the coordinator can mark completed.",
            json!({"task_id":{"type":"integer"},"state":{"type":"string","enum":["working","waiting","review","completed"]},"summary":{"type":"string"}}),
            &["task_id", "state", "summary"],
        ),
        tool(
            "memory_topics",
            "List derived topic collections across memories. Collections are many-to-many navigation, not task routing.",
            json!({"topic":{"type":"string"}}),
            &[],
        ),
    ];
    tools.push(tool(
        "memory_status",
        "Inspect background extraction backlog and hourly model-run budget.",
        json!({}),
        &[],
    ));
    if caller == MAIN {
        tools.extend([
        tool("memory_merge_topics","Combine synonymous derived topic labels. This does not move or resume any conversation.",json!({"from":{"type":"string"},"into":{"type":"string"}}),&["from","into"]),
        tool("memory_settings","Enable or pause background extraction; set the hourly model-run budget (0–240). Retrieval is always available.",json!({"enabled":{"type":"boolean"},"runs_per_hour":{"type":"integer","minimum":0,"maximum":240}}),&["enabled","runs_per_hour"]),
        tool("list_tasks","List existing work before interpreting an ambiguous continuation.",json!({}),&[]),
        tool("cancel_task","Request cancellation of a task's active and queued turns. Cancellation is asynchronous; inspect_task reports the final state.",json!({"task_id":{"type":"integer"}}),&["task_id"]),
        tool("create_task","Create an independent background task. Returns immediately with a stable task_id. Provide a unique request_key and reuse it only for retries of this same action.",json!({"request_key":{"type":"string"},"title":{"type":"string"},"instructions":{"type":"string"}}),&["request_key","title","instructions"]),
        tool("continue_task","Queue follow-up instructions for an explicitly identified task. Do not choose a task on topic similarity alone.",json!({"request_key":{"type":"string"},"task_id":{"type":"integer"},"instructions":{"type":"string"}}),&["request_key","task_id","instructions"]),
        tool("memory_forget","Forget a memory or a source and all derived notes. Source tombstones prevent archive reimport. Original conversation files remain historical records.",json!({"memory_id":{"type":"integer"},"source_id":{"type":"integer"}}),&[]),
    ])
    }
    tools
}

impl NativeStore {
    pub(crate) fn call(&self, caller: ThreadId, name: &str, args: &Value) -> Result<Value> {
        ensure!(
            tool_list(caller).iter().any(|t| t["name"] == name),
            "Tool unavailable in this session"
        );
        let text = |key: &str| -> Result<&str> {
            args[key]
                .as_str()
                .with_context(|| format!("{key} required"))
        };
        let id = |key: &str| -> Result<u64> {
            args[key]
                .as_u64()
                .with_context(|| format!("{key} required"))
        };
        match name {
            "memory_search" => self.search_report(
                caller,
                text("query")?,
                args["sources"].as_bool().unwrap_or(false),
                &super::retrieval::SearchFilter {
                    kind: args["kind"].as_str().unwrap_or("").to_owned(),
                    topic: args["topic"].as_str().unwrap_or("").to_lowercase(),
                    session: args["session"].as_str().unwrap_or("").to_owned(),
                },
            ),
            "memory_get" => {
                let ids = args["ids"]
                    .as_array()
                    .context("ids required")?
                    .iter()
                    .map(|v| v.as_u64().context("Invalid ID"))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(offset) = args["evidence_offset"].as_u64() {
                    self.get_memory_page(caller, &ids, offset)
                } else {
                    self.get_memory(caller, &ids)
                }
            }
            "memory_timeline" => self.timeline(caller, id("source_id")?),
            "memory_source" => self.read_source(
                caller,
                id("source_id")?,
                args["offset"].as_u64().unwrap_or(0).try_into()?,
            ),
            "memory_write" => Ok(json!({"memory_id":self.write_memory(caller,&args["note"])?})),
            "memory_forget" => {
                self.forget(
                    caller,
                    args["source_id"].as_u64(),
                    args["memory_id"].as_u64(),
                )?;
                Ok(json!({"forgotten":true}))
            }
            "memory_commit_job" => {
                ensure!(
                    args["notes"].is_array() ^ args["summary"].is_object(),
                    "Return either observations or a session summary"
                );
                self.commit_extraction(
                    id("job_id")?,
                    id("attempt")?,
                    args["notes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                    args.get("summary"),
                )?;
                Ok(json!({"committed":true}))
            }
            "memory_status" => {
                let embeddings = self.embedding_status()?;
                let db = self.db.lock().expect("native store");
                let mut stmt = db.prepare("SELECT state,count(*) FROM jobs GROUP BY state")?;
                let jobs = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
                    .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?;
                let mut stmt =
                    db.prepare("SELECT key,value FROM meta WHERE key LIKE 'observer_%'")?;
                let settings = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
                    .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?;
                Ok(json!({"jobs":jobs,"settings":settings,"embeddings":embeddings}))
            }
            "memory_settings" => {
                let enabled = args["enabled"].as_bool().context("enabled required")?;
                let budget = id("runs_per_hour")?;
                ensure!(budget <= 240, "Maximum 240 runs per hour");
                let mut db = self.db.lock().expect("native store");
                let tx = db.transaction()?;
                tx.execute(
                    "UPDATE meta SET value=? WHERE key='observer_enabled'",
                    [enabled as u64],
                )?;
                tx.execute(
                    "UPDATE meta SET value=? WHERE key='observer_budget'",
                    [budget],
                )?;
                tx.commit()?;
                Ok(json!({"enabled":enabled,"runs_per_hour":budget}))
            }
            "list_tasks" => self.tasks(),
            "cancel_task" => {
                let tid = id("task_id")?;
                ensure!(tid != MAIN.0 && tid != OBSERVER.0, "Invalid task scope");
                let count=self.db.lock().expect("native store").execute("UPDATE requests SET state='cancel_requested' WHERE thread_id=? AND state IN ('queued','connecting','dispatching','running')",[tid])?;
                Ok(json!({"cancellation_requested":count}))
            }
            "create_task" => Ok(
                json!({"task_id":self.create_task(text("request_key")?,text("title")?,text("instructions")?)?,"state":"queued"}),
            ),
            "continue_task" => {
                ensure!(id("task_id")? != 0, "Follow-up tasks must have a task ID");
                Ok(
                    json!({"request_id":self.enqueue(text("request_key")?,id("task_id")?,text("instructions")?)?,"state":"queued"}),
                )
            }
            "inspect_task" => {
                let tid = id("task_id")?;
                ensure!(
                    tid != OBSERVER.0 && (caller == MAIN || caller.0 == tid),
                    "Cannot inspect another task"
                );
                let db = self.db.lock().expect("native store");
                let mut task=db.query_row("SELECT name,state,summary FROM threads WHERE id=?",[tid],|r|Ok(json!({"task_id":tid,"title":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"summary":r.get::<_,String>(2)?})))?;
                let mut stmt=db.prepare("SELECT id,title,substr(body,1,3000) FROM sources WHERE thread_id=? AND forgotten=0 AND retired=0 ORDER BY id DESC LIMIT 4")?;
                task["recent_events"]=json!(stmt.query_map([tid],|r|Ok(json!({"source_id":r.get::<_,u64>(0)?,"title":r.get::<_,String>(1)?,"body":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?);
                Ok(task)
            }
            "update_task" => {
                let tid = id("task_id")?;
                let state = text("state")?;
                let summary = text("summary")?;
                ensure!(
                    tid != 0 && tid != OBSERVER.0 && (caller == MAIN || caller.0 == tid),
                    "Invalid task scope"
                );
                ensure!(
                    ["working", "waiting", "review", "completed"].contains(&state)
                        && summary.chars().count() <= 4000,
                    "Invalid task status"
                );
                ensure!(
                    state != "completed" || caller == MAIN,
                    "Task completion must be reviewed by the coordinator"
                );
                let mut db = self.db.lock().expect("native store");
                let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                if state == "completed" {
                    let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM requests WHERE thread_id=? AND state IN ('queued','connecting','dispatching','running','cancel_requested'))",[tid],|r|r.get(0))?;
                    ensure!(
                        !active,
                        "Task still has active requests; inspect before completing it"
                    );
                }
                let changed = tx.execute(
                    "UPDATE threads SET state=?,summary=?,revision=revision+1 WHERE id=?",
                    params![state, summary, tid],
                )?;
                ensure!(changed == 1, "Unknown task");
                Self::bump(&tx)?;
                tx.commit()?;
                Ok(json!({"updated":true}))
            }
            "memory_topics" => self.topics(caller, args["topic"].as_str()),
            "memory_merge_topics" => {
                self.merge_topics(text("from")?, text("into")?)?;
                Ok(json!({"merged":true}))
            }

            _ => bail!("Unknown tool"),
        }
    }
}
