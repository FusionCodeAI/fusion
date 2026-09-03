use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Attempts to parse a Mermaid diagram source and render it into ASCII/Unicode diagram art.
pub fn render_mermaid_ascii(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }

    let first_line = trimmed.lines().next().unwrap_or("").trim().to_lowercase();
    if first_line.starts_with("graph") || first_line.starts_with("flowchart") {
        render_flowchart_ascii(trimmed)
    } else if first_line.starts_with("sequencediagram") {
        render_sequence_ascii(trimmed)
    } else {
        None
    }
}

/// Represents a node in a flowchart.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowNode {
    id: String,
    label: String,
    shape: NodeShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeShape {
    Box,
    Round,
    Diamond,
    Database,
}

/// Represents an edge connecting two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowEdge {
    from: String,
    to: String,
    label: Option<String>,
    dashed: bool,
}

fn parse_node_spec(token: &str) -> Option<FlowNode> {
    let s = token.trim();
    if s.is_empty() {
        return None;
    }

    // Check for A[(Database)]
    if let Some((id, rest)) = s.split_once("[(") {
        if let Some((label, _)) = rest.split_once(")]") {
            return Some(FlowNode {
                id: id.trim().to_string(),
                label: label.trim().trim_matches('"').to_string(),
                shape: NodeShape::Database,
            });
        }
    }

    // Check for A{Diamond}
    if let Some((id, rest)) = s.split_once('{') {
        if let Some((label, _)) = rest.split_once('}') {
            return Some(FlowNode {
                id: id.trim().to_string(),
                label: label.trim().trim_matches('"').to_string(),
                shape: NodeShape::Diamond,
            });
        }
    }

    // Check for A(Round)
    if let Some((id, rest)) = s.split_once('(') {
        if let Some((label, _)) = rest.split_once(')') {
            return Some(FlowNode {
                id: id.trim().to_string(),
                label: label.trim().trim_matches('"').to_string(),
                shape: NodeShape::Round,
            });
        }
    }

    // Check for A[Box]
    if let Some((id, rest)) = s.split_once('[') {
        if let Some((label, _)) = rest.split_once(']') {
            return Some(FlowNode {
                id: id.trim().to_string(),
                label: label.trim().trim_matches('"').to_string(),
                shape: NodeShape::Box,
            });
        }
    }

    // Bare identifier
    Some(FlowNode {
        id: s.to_string(),
        label: s.to_string(),
        shape: NodeShape::Box,
    })
}

fn render_flowchart_ascii(source: &str) -> Option<String> {
    let mut nodes: HashMap<String, FlowNode> = HashMap::new();
    let mut edges: Vec<FlowEdge> = Vec::new();
    let mut direction = "TD";

    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("%%")
            || trimmed.starts_with("subgraph")
            || trimmed == "end"
            || trimmed.starts_with("classDef")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("style ")
        {
            continue;
        }

        if idx == 0 {
            let lower = trimmed.to_lowercase();
            if lower.contains(" lr") {
                direction = "LR";
            }
            continue;
        }

        // Look for arrows: -->, -.->, ==>, -->|label|, -- label -->
        let mut arrow_found = false;
        let arrow_patterns = [
            ("-.->", true),
            ("==>", false),
            ("-->", false),
            ("->", false),
        ];

        for &(arrow, is_dashed) in &arrow_patterns {
            if let Some((left_part, right_part)) = trimmed.split_once(arrow) {
                arrow_found = true;
                let left_str = left_part.trim();
                let mut right_str = right_part.trim();
                let mut edge_label = None;

                // Handle |label| on right part
                if right_str.starts_with('|') {
                    if let Some(rest) = right_str.strip_prefix('|') {
                        if let Some((lbl, remaining)) = rest.split_once('|') {
                            edge_label = Some(lbl.trim().to_string());
                            right_str = remaining.trim();
                        }
                    }
                }

                if let Some(from_node) = parse_node_spec(left_str) {
                    nodes.entry(from_node.id.clone()).or_insert(from_node.clone());
                    if let Some(to_node) = parse_node_spec(right_str) {
                        nodes.entry(to_node.id.clone()).or_insert(to_node.clone());
                        edges.push(FlowEdge {
                            from: from_node.id,
                            to: to_node.id,
                            label: edge_label,
                            dashed: is_dashed,
                        });
                    }
                }
                break;
            }
        }

        if !arrow_found {
            // Standalone node definition
            if let Some(node) = parse_node_spec(trimmed) {
                nodes.entry(node.id.clone()).or_insert(node);
            }
        }
    }

    if nodes.is_empty() {
        return None;
    }

    if direction == "LR" {
        render_lr_ascii(&nodes, &edges)
    } else {
        render_td_ascii(&nodes, &edges)
    }
}

fn render_box(label: &str, shape: NodeShape, width: usize) -> Vec<String> {
    let padding = width.saturating_sub(label.chars().count() + 2);
    let left_pad = padding / 2;
    let right_pad = padding - left_pad;

    let (top, mid_l, mid_r, bot) = match shape {
        NodeShape::Diamond => (
            format!("+{}+", "-".repeat(width.saturating_sub(2))),
            "<",
            ">",
            format!("+{}+", "-".repeat(width.saturating_sub(2))),
        ),
        NodeShape::Round => (
            format!(" ({}) ", "-".repeat(width.saturating_sub(4))),
            "(",
            ")",
            format!(" ({}) ", "-".repeat(width.saturating_sub(4))),
        ),
        NodeShape::Database => (
            format!("[({})]", "-".repeat(width.saturating_sub(4))),
            "|",
            "|",
            format!("[({})]", "-".repeat(width.saturating_sub(4))),
        ),
        NodeShape::Box => (
            format!("+{}+", "-".repeat(width.saturating_sub(2))),
            "|",
            "|",
            format!("+{}+", "-".repeat(width.saturating_sub(2))),
        ),
    };

    let mid = format!(
        "{}{}{}{}",
        mid_l,
        " ".repeat(left_pad + 1),
        label,
        format!("{}{}", " ".repeat(right_pad + 1), mid_r)
    );

    vec![top, mid, bot]
}

fn render_td_ascii(nodes: &HashMap<String, FlowNode>, edges: &[FlowEdge]) -> Option<String> {
    // Topological sort by layers
    let mut in_degrees: HashMap<String, usize> = HashMap::new();
    let mut adjacency: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();

    for id in nodes.keys() {
        in_degrees.insert(id.clone(), 0);
        adjacency.insert(id.clone(), Vec::new());
    }

    for edge in edges {
        *in_degrees.entry(edge.to.clone()).or_insert(0) += 1;
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push((edge.to.clone(), edge.label.clone()));
    }

    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let mut layers: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut visited: HashSet<String> = HashSet::new();

    for (id, &deg) in &in_degrees {
        if deg == 0 {
            queue.push_back((id.clone(), 0));
            visited.insert(id.clone());
        }
    }

    // If graph has cycles or no 0-in-degree nodes, pick the first
    if queue.is_empty() {
        if let Some(first_id) = nodes.keys().next() {
            queue.push_back((first_id.clone(), 0));
            visited.insert(first_id.clone());
        }
    }

    while let Some((id, layer)) = queue.pop_front() {
        layers.entry(layer).or_default().push(id.clone());
        if let Some(neighbors) = adjacency.get(&id) {
            for (next_id, _) in neighbors {
                if !visited.contains(next_id) {
                    visited.insert(next_id.clone());
                    queue.push_back((next_id.clone(), layer + 1));
                }
            }
        }
    }

    // Add any remaining disconnected nodes to layer 0
    for id in nodes.keys() {
        if !visited.contains(id) {
            layers.entry(0).or_default().push(id.clone());
        }
    }

    let mut out = String::new();

    // Box width: based on longest label
    let max_label_len = nodes
        .values()
        .map(|n| n.label.chars().count())
        .max()
        .unwrap_or(10)
        .max(12);
    let box_width = (max_label_len + 4).max(18);

    for (layer_idx, node_ids) in &layers {
        if *layer_idx > 0 {
            // Find edge label from previous layer if any
            let prev_nodes = layers.get(&(layer_idx - 1)).cloned().unwrap_or_default();
            let mut edge_label = None;
            for prev in &prev_nodes {
                if let Some(neighbors) = adjacency.get(prev) {
                    for (to_id, lbl) in neighbors {
                        if node_ids.contains(to_id) && lbl.is_some() {
                            edge_label = lbl.clone();
                            break;
                        }
                    }
                }
            }

            let center_indent = (box_width / 2).saturating_sub(1);
            if let Some(lbl) = edge_label {
                out.push_str(&format!(
                    "  {}| ({})\n",
                    " ".repeat(center_indent),
                    lbl
                ));
            } else {
                out.push_str(&format!("  {}|\n", " ".repeat(center_indent)));
            }
            out.push_str(&format!("  {}v\n", " ".repeat(center_indent)));
        }

        // Render nodes in this layer side-by-side
        let mut row_boxes: Vec<Vec<String>> = Vec::new();
        for id in node_ids {
            if let Some(node) = nodes.get(id) {
                row_boxes.push(render_box(&node.label, node.shape, box_width));
            }
        }

        for row_i in 0..3 {
            out.push_str("  ");
            for (b_idx, b) in row_boxes.iter().enumerate() {
                if b_idx > 0 {
                    out.push_str("    ");
                }
                out.push_str(&b[row_i]);
            }
            out.push('\n');
        }
    }

    Some(out)
}

fn render_lr_ascii(nodes: &HashMap<String, FlowNode>, edges: &[FlowEdge]) -> Option<String> {
    let mut out = String::new();
    let max_label_len = nodes
        .values()
        .map(|n| n.label.chars().count())
        .max()
        .unwrap_or(8);
    let box_width = (max_label_len + 4).max(14);

    let mut visited: HashSet<String> = HashSet::new();
    let mut ordered: Vec<String> = Vec::new();

    for edge in edges {
        if !visited.contains(&edge.from) {
            visited.insert(edge.from.clone());
            ordered.push(edge.from.clone());
        }
        if !visited.contains(&edge.to) {
            visited.insert(edge.to.clone());
            ordered.push(edge.to.clone());
        }
    }

    for id in nodes.keys() {
        if !visited.contains(id) {
            ordered.push(id.clone());
        }
    }

    let mut row_boxes: Vec<Vec<String>> = Vec::new();
    for id in &ordered {
        if let Some(node) = nodes.get(id) {
            row_boxes.push(render_box(&node.label, node.shape, box_width));
        }
    }

    for row_i in 0..3 {
        out.push_str("  ");
        for (idx, b) in row_boxes.iter().enumerate() {
            if idx > 0 {
                if row_i == 1 {
                    out.push_str(" ----> ");
                } else {
                    out.push_str("       ");
                }
            }
            out.push_str(&b[row_i]);
        }
        out.push('\n');
    }

    Some(out)
}

fn render_sequence_ascii(source: &str) -> Option<String> {
    let mut participants: Vec<String> = Vec::new();
    let mut messages: Vec<(String, String, String, bool)> = Vec::new(); // (from, to, text, is_dotted)

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("%%") || trimmed.eq_ignore_ascii_case("sequencediagram") {
            continue;
        }

        if trimmed.starts_with("participant ") {
            let part = trimmed.trim_start_matches("participant ").trim();
            let name = if let Some((_, alias)) = part.split_once(" as ") {
                alias.trim()
            } else {
                part
            };
            if !participants.contains(&name.to_string()) {
                participants.push(name.to_string());
            }
            continue;
        }

        // Check arrows: ->>, -->>, ->, -->
        let arrows = [
            ("-->>", true),
            ("->>", false),
            ("-->", true),
            ("->", false),
        ];

        for &(arrow, is_dotted) in &arrows {
            if let Some((left, right)) = trimmed.split_once(arrow) {
                let from = left.trim().to_string();
                let (to, msg) = if let Some((to_part, msg_part)) = right.split_once(':') {
                    (to_part.trim().to_string(), msg_part.trim().to_string())
                } else {
                    (right.trim().to_string(), String::new())
                };

                if !participants.contains(&from) {
                    participants.push(from.clone());
                }
                if !participants.contains(&to) {
                    participants.push(to.clone());
                }
                messages.push((from, to, msg, is_dotted));
                break;
            }
        }
    }

    if participants.is_empty() {
        return None;
    }

    let col_width: usize = 18;
    let mut out = String::new();

    // 1. Participant headers
    out.push_str("  ");
    for p in &participants {
        let pad = col_width.saturating_sub(p.chars().count());
        out.push_str(p);
        out.push_str(&" ".repeat(pad));
    }
    out.push('\n');

    // 2. Lifeline tops
    out.push_str("  ");
    for _ in &participants {
        let center = col_width / 2;
        out.push_str(&" ".repeat(center));
        out.push('|');
        out.push_str(&" ".repeat(col_width.saturating_sub(center + 1)));
    }
    out.push('\n');

    // 3. Messages
    for (from, to, msg, _) in &messages {
        let from_idx = participants.iter().position(|p| p == from).unwrap_or(0);
        let to_idx = participants.iter().position(|p| p == to).unwrap_or(0);

        let from_center = from_idx * col_width + col_width / 2;
        let to_center = to_idx * col_width + col_width / 2;

        let (left_c, right_c, is_left_to_right) = if from_center < to_center {
            (from_center, to_center, true)
        } else {
            (to_center, from_center, false)
        };

        // Text line
        out.push_str("  ");
        out.push_str(&" ".repeat(left_c));
        out.push('|');
        let span = right_c.saturating_sub(left_c + 1);
        let pad = span.saturating_sub(msg.chars().count() + 4);
        let label_line = format!("-- {} --", msg);
        out.push_str(&label_line);
        out.push_str(&"-".repeat(pad));
        if is_left_to_right {
            out.push('>');
        } else {
            out.push('|');
        }
        out.push('\n');
    }

    // 4. Lifeline bottoms
    out.push_str("  ");
    for _ in &participants {
        let center = col_width / 2;
        out.push_str(&" ".repeat(center));
        out.push('|');
        out.push_str(&" ".repeat(col_width.saturating_sub(center + 1)));
    }
    out.push('\n');

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flowchart_td_rendering() {
        let src = "graph TD\n    A[Start] --> B[Process]\n    B --> C[End]";
        let ascii = render_mermaid_ascii(src).expect("should render ascii flowchart");
        assert!(ascii.contains("+---------"));
        assert!(ascii.contains("Start"));
        assert!(ascii.contains("Process"));
        assert!(ascii.contains("End"));
        assert!(ascii.contains("v"));
    }

    #[test]
    fn test_flowchart_lr_rendering() {
        let src = "graph LR\n    A --> B --> C";
        let ascii = render_mermaid_ascii(src).expect("should render ascii flowchart");
        assert!(ascii.contains("---->"));
        assert!(ascii.contains("A"));
        assert!(ascii.contains("B"));
        assert!(ascii.contains("C"));
    }

    #[test]
    fn test_sequence_diagram_rendering() {
        let src = "sequenceDiagram\n    User->>Server: Request\n    Server-->>User: Response";
        let ascii = render_mermaid_ascii(src).expect("should render ascii sequence diagram");
        assert!(ascii.contains("User"));
        assert!(ascii.contains("Server"));
        assert!(ascii.contains("Request"));
        assert!(ascii.contains("Response"));
    }
}
