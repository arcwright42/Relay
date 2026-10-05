use super::*;

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}

pub(crate) fn task_tools(caller: ThreadId) -> Vec<Value> {
    if caller == RETIRED_OBSERVER {
        return vec![];
    }
    let mut tools = vec![
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
    ];
    if caller == MAIN {
        tools.extend([
            tool("list_tasks", "List existing work before interpreting an ambiguous continuation.", json!({}), &[]),
            tool("cancel_task", "Request cancellation of a task's active and queued turns. Cancellation is asynchronous; inspect_task reports the final state.", json!({"task_id":{"type":"integer"}}), &["task_id"]),
            tool("create_task", "Create an independent background task. Returns immediately with a stable task_id. Provide a unique request_key and reuse it only for retries of this same action.", json!({"request_key":{"type":"string"},"title":{"type":"string"},"instructions":{"type":"string"}}), &["request_key","title","instructions"]),
            tool("continue_task", "Queue follow-up instructions for an explicitly identified task. Do not choose a task on topic similarity alone.", json!({"request_key":{"type":"string"},"task_id":{"type":"integer"},"instructions":{"type":"string"}}), &["request_key","task_id","instructions"]),
        ]);
    }
    tools
}

impl ResidentStore {
    pub(crate) fn call_task(&self, caller: ThreadId, name: &str, args: &Value) -> Result<Value> {
        ensure!(
            task_tools(caller).iter().any(|t| t["name"] == name),
            "Task tool unavailable in this session"
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
            "list_tasks" => self.tasks(),
            "cancel_task" => {
                let tid = id("task_id")?;
                ensure!(
                    tid != MAIN.0 && tid != RETIRED_OBSERVER.0,
                    "Invalid task scope"
                );
                let count=self.db.lock().expect("resident store").execute("UPDATE requests SET state='cancel_requested' WHERE thread_id=? AND state IN ('queued','connecting','dispatching','running')",[tid])?;
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
                    tid != RETIRED_OBSERVER.0 && (caller == MAIN || caller.0 == tid),
                    "Cannot inspect another task"
                );
                let db = self.db.lock().expect("resident store");
                let mut task=db.query_row("SELECT name,state,summary FROM threads WHERE id=?",[tid],|r|Ok(json!({"task_id":tid,"title":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"summary":r.get::<_,String>(2)?})))?;
                drop(db);
                let saved = crate::store::load(&self.root, ThreadId(tid))?;
                task["recent_events"]=json!(saved.messages.iter().rev().take(4).map(|m|json!({"message_id":m.id,"role":m.role,"body":m.text.chars().take(3000).collect::<String>()})).collect::<Vec<_>>());
                Ok(task)
            }
            "update_task" => {
                let tid = id("task_id")?;
                let state = text("state")?;
                let summary = text("summary")?;
                ensure!(
                    tid != 0 && tid != RETIRED_OBSERVER.0 && (caller == MAIN || caller.0 == tid),
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
                let mut db = self.db.lock().expect("resident store");
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
            _ => bail!("Unknown task tool"),
        }
    }
}
