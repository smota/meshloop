use std::collections::{HashMap, HashSet};

use petgraph::algo::toposort;
use petgraph::graphmap::DiGraphMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    Tier1,
    Tier2,
    Tier3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: TaskId,
    pub description: String,
    pub depends_on: Vec<TaskId>,
    pub tier: Option<Tier>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub empty_diff_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    pub graph_id: String,
    pub nodes: Vec<TaskNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    Cycle(Vec<TaskId>),
    DanglingDependency { node: TaskId, missing: TaskId },
    DuplicateId(TaskId),
    Empty,
    IllegalGraphId(String),
    ReservedTaskId(TaskId),
}

/// `graph_id` values that are safe as Git branch segments and directory names.
pub fn graph_id_is_legal(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

impl TaskGraph {
    /// Validates structure only (cycles, dangling deps, duplicates) — never decomposition
    /// quality, which runtime-design.md §5 states cannot be mechanically checked.
    pub fn validate(&self) -> Result<(), GraphError> {
        self.try_topological_order().map(|_| ())
    }

    /// Computes a valid topological order using petgraph's iterative cycle-safe algorithm.
    /// Returns GraphError if the graph is empty, contains duplicate IDs, dangling dependencies,
    /// or directed cycles.
    pub fn try_topological_order(&self) -> Result<Vec<TaskId>, GraphError> {
        if self.nodes.is_empty() {
            return Err(GraphError::Empty);
        }
        if !graph_id_is_legal(&self.graph_id) {
            return Err(GraphError::IllegalGraphId(self.graph_id.clone()));
        }

        let mut seen = HashSet::new();
        for node in &self.nodes {
            if node.id.0 == 0 {
                return Err(GraphError::ReservedTaskId(node.id));
            }
            if !seen.insert(node.id) {
                return Err(GraphError::DuplicateId(node.id));
            }
        }

        let by_id: HashMap<TaskId, &TaskNode> = self.nodes.iter().map(|n| (n.id, n)).collect();
        for node in &self.nodes {
            for dep in &node.depends_on {
                if !by_id.contains_key(dep) {
                    return Err(GraphError::DanglingDependency {
                        node: node.id,
                        missing: *dep,
                    });
                }
            }
        }

        let mut graph = DiGraphMap::<TaskId, ()>::new();
        for node in &self.nodes {
            graph.add_node(node.id);
        }
        for node in &self.nodes {
            for dep in &node.depends_on {
                // Dependency `*dep` must precede `node.id`
                graph.add_edge(*dep, node.id, ());
            }
        }

        match toposort(&graph, None) {
            Ok(order) => Ok(order),
            Err(cycle) => {
                let start_node = cycle.node_id();
                let mut path = vec![start_node];
                let mut visited = HashSet::new();
                let mut curr = start_node;
                while visited.insert(curr) {
                    if let Some(next) = graph.neighbors(curr).next() {
                        path.push(next);
                        if next == start_node {
                            break;
                        }
                        curr = next;
                    } else {
                        break;
                    }
                }
                Err(GraphError::Cycle(path))
            }
        }
    }

    /// Nodes with all dependencies already in `integrated`, per execution-lifecycle.md's
    /// `pending -> ready` precondition. Does not itself schedule anything.
    pub fn ready_nodes<'a>(&'a self, integrated: &HashSet<TaskId>) -> Vec<&'a TaskNode> {
        self.nodes
            .iter()
            .filter(|n| !integrated.contains(&n.id))
            .filter(|n| n.depends_on.iter().all(|d| integrated.contains(d)))
            .collect()
    }

    /// A single valid dependency order. Cycle-safe: returns empty vector on cyclic graph.
    pub fn topological_order(&self) -> Vec<TaskId> {
        self.try_topological_order().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, deps: &[u32]) -> TaskNode {
        TaskNode {
            id: TaskId(id),
            description: format!("task {id}"),
            depends_on: deps.iter().map(|d| TaskId(*d)).collect(),
            tier: None,
            allowed_paths: vec![],
            empty_diff_ok: false,
        }
    }

    #[test]
    fn rejects_empty_graph() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![],
        };
        assert_eq!(g.validate(), Err(GraphError::Empty));
    }

    #[test]
    fn rejects_dangling_dependency() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[2])],
        };
        assert_eq!(
            g.validate(),
            Err(GraphError::DanglingDependency {
                node: TaskId(1),
                missing: TaskId(2)
            })
        );
    }

    #[test]
    fn rejects_duplicate_id() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[]), node(1, &[])],
        };
        assert_eq!(g.validate(), Err(GraphError::DuplicateId(TaskId(1))));
    }

    #[test]
    fn rejects_direct_cycle() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[2]), node(2, &[1])],
        };
        assert!(matches!(g.validate(), Err(GraphError::Cycle(_))));
    }

    #[test]
    fn rejects_self_cycle() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[1])],
        };
        assert!(matches!(g.validate(), Err(GraphError::Cycle(_))));
    }

    #[test]
    fn accepts_valid_dag() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[]), node(2, &[1]), node(3, &[1, 2])],
        };
        assert_eq!(g.validate(), Ok(()));
    }

    #[test]
    fn topological_order_never_places_a_dependency_after_its_dependent() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[]), node(2, &[1]), node(3, &[1, 2])],
        };
        let order = g.topological_order();
        assert_eq!(order.len(), 3);
        let pos = |id: u32| order.iter().position(|t| t.0 == id).unwrap();
        assert!(pos(1) < pos(2));
        assert!(pos(2) < pos(3));
    }

    #[test]
    fn rejects_reserved_task_id_zero() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(0, &[])],
        };
        assert_eq!(g.validate(), Err(GraphError::ReservedTaskId(TaskId(0))));
    }

    #[test]
    fn rejects_illegal_graph_id() {
        let g = TaskGraph {
            graph_id: "bad/id".into(),
            nodes: vec![node(1, &[])],
        };
        assert!(matches!(g.validate(), Err(GraphError::IllegalGraphId(_))));
    }

    #[test]
    fn ready_nodes_respects_dependencies() {
        let g = TaskGraph {
            graph_id: "g".into(),
            nodes: vec![node(1, &[]), node(2, &[1]), node(3, &[1, 2])],
        };
        let none = HashSet::new();
        let ready_ids: Vec<u32> = g.ready_nodes(&none).iter().map(|n| n.id.0).collect();
        assert_eq!(ready_ids, vec![1]);

        let mut one_done = HashSet::new();
        one_done.insert(TaskId(1));
        let ready_ids: Vec<u32> = g.ready_nodes(&one_done).iter().map(|n| n.id.0).collect();
        assert_eq!(ready_ids, vec![2]);
    }

    #[test]
    fn deep_pipeline_dag_sorts_without_stack_overflow() {
        let n = 1000;
        let mut nodes = Vec::with_capacity(n);
        nodes.push(node(1, &[]));
        for i in 2..=n as u32 {
            nodes.push(node(i, &[i - 1]));
        }
        let g = TaskGraph {
            graph_id: "deep-chain".into(),
            nodes,
        };
        let order = g.try_topological_order().expect("valid deep chain");
        assert_eq!(order.len(), n);
        assert_eq!(order.first().unwrap().0, 1);
        assert_eq!(order.last().unwrap().0, n as u32);
    }

    #[test]
    fn try_topological_order_detects_triangle_cycle_safely() {
        let g = TaskGraph {
            graph_id: "triangle-cycle".into(),
            nodes: vec![node(1, &[3]), node(2, &[1]), node(3, &[2])],
        };
        assert!(matches!(
            g.try_topological_order(),
            Err(GraphError::Cycle(_))
        ));
        assert_eq!(g.topological_order(), Vec::<TaskId>::new());
    }
}
