use serde::{Deserialize, Serialize};

/// Status of an individual task in the todo tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Task is queued and has not yet started.
    Pending,
    /// Task is actively being executed.
    InProgress,
    /// Task has finished successfully.
    Completed,
    /// Task is blocked waiting on an external condition or dependency.
    Blocked,
    /// Task was cancelled or dropped.
    Dropped,
}

impl TodoStatus {
    /// Returns true if the task is pending.
    pub fn is_pending(&self) -> bool {
        matches!(self, TodoStatus::Pending)
    }

    /// Returns true if the task is actively in progress.
    pub fn is_in_progress(&self) -> bool {
        matches!(self, TodoStatus::InProgress)
    }

    /// Returns true if the task has completed.
    pub fn is_completed(&self) -> bool {
        matches!(self, TodoStatus::Completed)
    }

    /// Returns true if the task is blocked.
    pub fn is_blocked(&self) -> bool {
        matches!(self, TodoStatus::Blocked)
    }

    /// Returns true if the task has been dropped.
    pub fn is_dropped(&self) -> bool {
        matches!(self, TodoStatus::Dropped)
    }

    /// Returns a standardized markdown/TUI checklist checkbox symbol.
    pub fn symbol(&self) -> &'static str {
        match self {
            TodoStatus::Completed => "[x]",
            TodoStatus::InProgress => "[>]",
            TodoStatus::Pending => "[ ]",
            TodoStatus::Blocked => "[!]",
            TodoStatus::Dropped => "[-]",
        }
    }

    /// Human-readable label for the status.
    pub fn label(&self) -> &'static str {
        match self {
            TodoStatus::Completed => "completed",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Pending => "pending",
            TodoStatus::Blocked => "blocked",
            TodoStatus::Dropped => "dropped",
        }
    }
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// An individual actionable task item within a phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: usize,
    pub task: String,
    pub status: TodoStatus,
    pub phase: String,
    pub reason: Option<String>,
    pub created_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

impl TodoItem {
    /// Creates a new pending task item.
    pub fn new(id: usize, task: impl Into<String>, phase: impl Into<String>) -> Self {
        Self {
            id,
            task: task.into(),
            status: TodoStatus::Pending,
            phase: phase.into(),
            reason: None,
            created_at_ms: current_timestamp_ms(),
            completed_at_ms: None,
        }
    }
}

/// A logical execution phase grouping related tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPhase {
    pub name: String,
    pub items: Vec<TodoItem>,
}

impl TodoPhase {
    /// Creates a new empty phase with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            items: Vec::new(),
        }
    }
}

/// Pure-Rust task tracker managing phased execution and task states
/// according to arXiv:2608.26263 execution state & phased progress models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoState {
    pub phases: Vec<TodoPhase>,
    pub next_id: usize,
}

impl Default for TodoState {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoState {
    /// Creates a new, empty `TodoState`.
    pub fn new() -> Self {
        Self {
            phases: Vec::new(),
            next_id: 1,
        }
    }

    /// Replaces existing phases with new phases and tasks, marking the
    /// earliest pending task as `InProgress`.
    pub fn init(&mut self, list: Vec<(String, Vec<String>)>) {
        self.phases.clear();
        self.next_id = 1;
        let now = current_timestamp_ms();

        for (phase_name, tasks) in list {
            let mut items = Vec::with_capacity(tasks.len());
            for task in tasks {
                items.push(TodoItem {
                    id: self.next_id,
                    task,
                    status: TodoStatus::Pending,
                    phase: phase_name.clone(),
                    reason: None,
                    created_at_ms: now,
                    completed_at_ms: None,
                });
                self.next_id += 1;
            }
            self.phases.push(TodoPhase {
                name: phase_name,
                items,
            });
        }

        // Promote earliest pending task to InProgress
        self.promote_earliest_pending();
    }

    /// Sets the task matching `task_query` (by exact text or substring) to `InProgress`,
    /// demoting other in-progress tasks to pending.
    pub fn start(&mut self, task_query: &str) -> bool {
        let Some((target_p, target_i)) = self.find_matching_item_coords(task_query) else {
            return false;
        };

        // Demote all other currently in-progress tasks
        for (p_idx, phase) in self.phases.iter_mut().enumerate() {
            for (i_idx, item) in phase.items.iter_mut().enumerate() {
                if item.status == TodoStatus::InProgress && (p_idx, i_idx) != (target_p, target_i) {
                    item.status = TodoStatus::Pending;
                }
            }
        }

        // Activate target task
        let target = &mut self.phases[target_p].items[target_i];
        target.status = TodoStatus::InProgress;
        target.reason = None;
        true
    }

    /// Marks matching task or entire phase as `Completed`. Automatically
    /// promotes the next earliest `Pending` task to `InProgress`.
    pub fn done(&mut self, query: &str) -> bool {
        let q = query.trim();
        if q.is_empty() {
            return false;
        }

        let now = current_timestamp_ms();

        // Check if query is an explicit ID match first (#1 or 1)
        let is_id_query = q
            .strip_prefix('#')
            .unwrap_or(q)
            .trim()
            .parse::<usize>()
            .is_ok();

        if !is_id_query {
            // 1. Check if query matches an entire phase by exact name (case-insensitive)
            if let Some(phase_idx) = self
                .phases
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(q))
            {
                let mut marked_any = false;
                for item in &mut self.phases[phase_idx].items {
                    if item.status != TodoStatus::Completed && item.status != TodoStatus::Dropped {
                        item.status = TodoStatus::Completed;
                        item.completed_at_ms = Some(now);
                        marked_any = true;
                    }
                }

                if !self.has_in_progress_task() {
                    self.promote_earliest_pending();
                }
                return marked_any || !self.phases[phase_idx].items.is_empty();
            }
        }

        // 2. Check if query matches a single task
        if let Some((p_idx, i_idx)) = self.find_matching_item_coords(query) {
            let was_in_progress = self.phases[p_idx].items[i_idx].status == TodoStatus::InProgress;
            self.phases[p_idx].items[i_idx].status = TodoStatus::Completed;
            self.phases[p_idx].items[i_idx].completed_at_ms = Some(now);

            // If task was in-progress, or no tasks are currently in progress, promote next pending
            if was_in_progress || !self.has_in_progress_task() {
                self.promote_earliest_pending();
            }
            return true;
        }

        // 3. Check if query matches a phase as a substring (if no task matched)
        let q_lower = q.to_lowercase();
        if let Some(phase_idx) = self
            .phases
            .iter()
            .position(|p| p.name.to_lowercase().contains(&q_lower))
        {
            let mut marked_any = false;
            for item in &mut self.phases[phase_idx].items {
                if item.status != TodoStatus::Completed && item.status != TodoStatus::Dropped {
                    item.status = TodoStatus::Completed;
                    item.completed_at_ms = Some(now);
                    marked_any = true;
                }
            }

            if !self.has_in_progress_task() {
                self.promote_earliest_pending();
            }
            return marked_any || !self.phases[phase_idx].items.is_empty();
        }

        false
    }

    /// Marks task as `Blocked`, handing `InProgress` to the next pending task.
    pub fn block(&mut self, query: &str, reason: Option<String>) -> bool {
        let Some((p_idx, i_idx)) = self.find_matching_item_coords(query) else {
            return false;
        };

        let was_in_progress = self.phases[p_idx].items[i_idx].status == TodoStatus::InProgress;
        self.phases[p_idx].items[i_idx].status = TodoStatus::Blocked;
        self.phases[p_idx].items[i_idx].reason = reason;

        if was_in_progress || !self.has_in_progress_task() {
            self.promote_earliest_pending();
        }

        true
    }

    /// Moves a blocked task back to `Pending`.
    pub fn unblock(&mut self, query: &str) -> bool {
        // First look for a blocked item matching the query
        if let Some((p_idx, i_idx)) = self.find_blocked_item_coords(query) {
            let item = &mut self.phases[p_idx].items[i_idx];
            item.status = TodoStatus::Pending;
            item.reason = None;
            return true;
        }

        // Fallback: check if matching item is blocked
        if let Some((p_idx, i_idx)) = self.find_matching_item_coords(query) {
            let item = &mut self.phases[p_idx].items[i_idx];
            if item.status == TodoStatus::Blocked {
                item.status = TodoStatus::Pending;
                item.reason = None;
                return true;
            }
        }

        false
    }

    /// Marks task as `Dropped`. If it was in progress, hands `InProgress` to next pending.
    pub fn drop_task(&mut self, query: &str) -> bool {
        let Some((p_idx, i_idx)) = self.find_matching_item_coords(query) else {
            return false;
        };

        let was_in_progress = self.phases[p_idx].items[i_idx].status == TodoStatus::InProgress;
        self.phases[p_idx].items[i_idx].status = TodoStatus::Dropped;

        if was_in_progress || !self.has_in_progress_task() {
            self.promote_earliest_pending();
        }

        true
    }

    /// Appends tasks to named phase (creating phase if not present).
    pub fn append(&mut self, phase: &str, items: Vec<String>) {
        let now = current_timestamp_ms();
        let phase_name = phase.trim().to_string();

        let phase_idx = match self
            .phases
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&phase_name))
        {
            Some(idx) => idx,
            None => {
                self.phases.push(TodoPhase {
                    name: phase_name.clone(),
                    items: Vec::new(),
                });
                self.phases.len() - 1
            }
        };

        let phase_title = self.phases[phase_idx].name.clone();
        for task in items {
            self.phases[phase_idx].items.push(TodoItem {
                id: self.next_id,
                task,
                status: TodoStatus::Pending,
                phase: phase_title.clone(),
                reason: None,
                created_at_ms: now,
                completed_at_ms: None,
            });
            self.next_id += 1;
        }
    }

    /// Clears all tasks and resets the ID counter.
    pub fn clear(&mut self) {
        self.phases.clear();
        self.next_id = 1;
    }

    /// Renders clean formatted plain-text/markdown checklist with summary count `[completed/total]`.
    pub fn view(&self) -> String {
        let total = self.total_count();
        let completed = self.completed_count();

        if self.phases.is_empty() || total == 0 {
            return format!("# Tasks [{completed}/{total}]\n\nNo tasks scheduled.\n");
        }

        let mut out = String::new();
        out.push_str(&format!("# Tasks [{completed}/{total}]\n\n"));

        for phase in &self.phases {
            out.push_str(&format!("## {}\n", phase.name));
            if phase.items.is_empty() {
                out.push_str("  (no tasks)\n\n");
                continue;
            }

            for item in &phase.items {
                let symbol = item.status.symbol();
                match item.status {
                    TodoStatus::Completed => {
                        out.push_str(&format!("- {symbol} #{}: {}\n", item.id, item.task));
                    }
                    TodoStatus::InProgress => {
                        out.push_str(&format!(
                            "- {symbol} #{}: {} (in progress)\n",
                            item.id, item.task
                        ));
                    }
                    TodoStatus::Blocked => {
                        if let Some(reason) = &item.reason {
                            out.push_str(&format!(
                                "- {symbol} #{}: {} (blocked: {})\n",
                                item.id, item.task, reason
                            ));
                        } else {
                            out.push_str(&format!(
                                "- {symbol} #{}: {} (blocked)\n",
                                item.id, item.task
                            ));
                        }
                    }
                    TodoStatus::Dropped => {
                        out.push_str(&format!(
                            "- {symbol} #{}: {} (dropped)\n",
                            item.id, item.task
                        ));
                    }
                    TodoStatus::Pending => {
                        out.push_str(&format!("- {symbol} #{}: {}\n", item.id, item.task));
                    }
                }
            }
            out.push('\n');
        }

        out
    }

    /// Returns a single line for TUI rail e.g. `[2/5 done] Current: Implement todo_state.rs`.
    pub fn compact_summary(&self) -> String {
        let total = self.total_count();
        let completed = self.completed_count();

        if let Some(current) = self.current_task() {
            format!("[{completed}/{total} done] Current: {}", current.task)
        } else if total > 0 && completed == total {
            format!("[{completed}/{total} done] All tasks completed")
        } else if total > 0 {
            format!("[{completed}/{total} done] No active task")
        } else {
            "[0/0 done] No tasks".to_string()
        }
    }

    /// Returns true if there are no tasks in any phase.
    pub fn is_empty(&self) -> bool {
        self.total_count() == 0
    }

    /// Returns the total count of all tasks across all phases.
    pub fn total_count(&self) -> usize {
        self.phases.iter().map(|p| p.items.len()).sum()
    }

    /// Returns the count of tasks that are in `Completed` status.
    pub fn completed_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .filter(|item| item.status == TodoStatus::Completed)
            .count()
    }

    /// Returns the count of tasks that are in `Pending` status.
    pub fn pending_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .filter(|item| item.status == TodoStatus::Pending)
            .count()
    }

    /// Returns the count of tasks that are in `InProgress` status.
    pub fn in_progress_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .filter(|item| item.status == TodoStatus::InProgress)
            .count()
    }

    /// Returns the count of tasks that are in `Blocked` status.
    pub fn blocked_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .filter(|item| item.status == TodoStatus::Blocked)
            .count()
    }

    /// Returns the count of tasks that are in `Dropped` status.
    pub fn dropped_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .filter(|item| item.status == TodoStatus::Dropped)
            .count()
    }

    /// Returns a reference to the currently active (`InProgress`) task, if any.
    pub fn current_task(&self) -> Option<&TodoItem> {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .find(|item| item.status == TodoStatus::InProgress)
    }

    /// Returns true if at least one task is currently `InProgress`.
    pub fn has_in_progress_task(&self) -> bool {
        self.current_task().is_some()
    }

    /// Promotes the next earliest `Pending` task to `InProgress`.
    /// Returns true if a task was promoted.
    pub fn promote_earliest_pending(&mut self) -> bool {
        for phase in &mut self.phases {
            for item in &mut phase.items {
                if item.status == TodoStatus::Pending {
                    item.status = TodoStatus::InProgress;
                    return true;
                }
            }
        }
        false
    }

    /// Finds task coordinates `(phase_index, item_index)` matching query.
    /// Priority:
    /// 1. Exact numeric ID (#1, 1)
    /// 2. Exact task name (case-sensitive)
    /// 3. Exact task name (case-insensitive)
    /// 4. Substring in task name (case-insensitive)
    fn find_matching_item_coords(&self, query: &str) -> Option<(usize, usize)> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }

        // 1. Exact ID match
        let id_str = q.strip_prefix('#').unwrap_or(q).trim();
        if let Ok(id) = id_str.parse::<usize>() {
            for (p_idx, phase) in self.phases.iter().enumerate() {
                for (i_idx, item) in phase.items.iter().enumerate() {
                    if item.id == id {
                        return Some((p_idx, i_idx));
                    }
                }
            }
        }

        // 2. Exact match (case-sensitive)
        for (p_idx, phase) in self.phases.iter().enumerate() {
            for (i_idx, item) in phase.items.iter().enumerate() {
                if item.task == q {
                    return Some((p_idx, i_idx));
                }
            }
        }

        // 3. Exact match (case-insensitive)
        for (p_idx, phase) in self.phases.iter().enumerate() {
            for (i_idx, item) in phase.items.iter().enumerate() {
                if item.task.eq_ignore_ascii_case(q) {
                    return Some((p_idx, i_idx));
                }
            }
        }

        // 4. Substring match (case-insensitive)
        let q_lower = q.to_lowercase();
        for (p_idx, phase) in self.phases.iter().enumerate() {
            for (i_idx, item) in phase.items.iter().enumerate() {
                if item.task.to_lowercase().contains(&q_lower) {
                    return Some((p_idx, i_idx));
                }
            }
        }

        None
    }

    /// Finds coordinates of a `Blocked` item matching the query.
    fn find_blocked_item_coords(&self, query: &str) -> Option<(usize, usize)> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }

        // Exact ID match on blocked item
        let id_str = q.strip_prefix('#').unwrap_or(q).trim();
        if let Ok(id) = id_str.parse::<usize>() {
            for (p_idx, phase) in self.phases.iter().enumerate() {
                for (i_idx, item) in phase.items.iter().enumerate() {
                    if item.id == id && item.status == TodoStatus::Blocked {
                        return Some((p_idx, i_idx));
                    }
                }
            }
        }

        // Exact / substring match on blocked items
        let q_lower = q.to_lowercase();
        for (p_idx, phase) in self.phases.iter().enumerate() {
            for (i_idx, item) in phase.items.iter().enumerate() {
                if item.status == TodoStatus::Blocked
                    && (item.task.eq_ignore_ascii_case(q)
                        || item.task.to_lowercase().contains(&q_lower))
                {
                    return Some((p_idx, i_idx));
                }
            }
        }

        None
    }

    /// Look up an item by numeric ID.
    pub fn get_task_by_id(&self, id: usize) -> Option<&TodoItem> {
        self.phases
            .iter()
            .flat_map(|p| &p.items)
            .find(|item| item.id == id)
    }

    /// Look up a mutable item by numeric ID.
    pub fn get_task_by_id_mut(&mut self, id: usize) -> Option<&mut TodoItem> {
        for phase in &mut self.phases {
            for item in &mut phase.items {
                if item.id == id {
                    return Some(item);
                }
            }
        }
        None
    }

    /// Look up an item by query string (exact match, ID, or substring).
    pub fn get_task(&self, query: &str) -> Option<&TodoItem> {
        self.find_matching_item_coords(query)
            .map(|(p, i)| &self.phases[p].items[i])
    }

    /// Returns an iterator over references to all items across all phases.
    pub fn all_items(&self) -> impl Iterator<Item = &TodoItem> {
        self.phases.iter().flat_map(|p| &p.items)
    }
}

/// Helper function to retrieve the current UNIX epoch timestamp in milliseconds.
fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
