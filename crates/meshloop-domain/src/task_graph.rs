use std::collections::{HashMap, HashSet};

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
}

impl TaskGraph {
    /// Validates structure only (cycles, dangling deps, duplicates) — never decomposition
    /// quality, which runtime-design.md §5 states cannot be mechanically checked.
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.nodes.is_empty() {
            return Err(GraphError::Empty);
        }

        let mut seen = HashSet::new();
        for node in &self.nodes {
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

        self.find_cycle(&by_id)
    }

    fn find_cycle(&self, by_id: &HashMap<TaskId, &TaskNode>) -> Result<(), GraphError> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            InProgress,
            Done,
        }

        let mut marks: HashMap<TaskId, Mark> =
            self.nodes.iter().map(|n| (n.id, Mark::Unvisited)).collect();
        let mut path = Vec::new();

        fn visit(
            id: TaskId,
            by_id: &HashMap<TaskId, &TaskNode>,
            marks: &mut HashMap<TaskId, Mark>,
            path: &mut Vec<TaskId>,
        ) -> Result<(), GraphError> {
            match marks[&id] {
                Mark::Done => return Ok(()),
                Mark::InProgress => {
                    let start = path.iter().position(|x| *x == id).unwrap_or(0);
                    let mut cycle = path[start..].to_vec();
                    cycle.push(id);
                    return Err(GraphError::Cycle(cycle));
                }
                Mark::Unvisited => {}
            }
            marks.insert(id, Mark::InProgress);
            path.push(id);
            for dep in &by_id[&id].depends_on {
                visit(*dep, by_id, marks, path)?;
            }
            path.pop();
            marks.insert(id, Mark::Done);
            Ok(())
        }

        for node in &self.nodes {
            visit(node.id, by_id, &mut marks, &mut path)?;
        }
        Ok(())
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

    /// A single valid dependency order. Only meaningful on an already-`validate`d graph —
    /// this does not re-check for cycles and will loop forever on one.
    pub fn topological_order(&self) -> Vec<TaskId> {
        let mut done = HashSet::new();
        let mut order = Vec::with_capacity(self.nodes.len());
        while order.len() < self.nodes.len() {
            for node in self.ready_nodes(&done) {
                if !done.contains(&node.id) {
                    order.push(node.id);
                }
            }
            for id in &order {
                done.insert(*id);
            }
        }
        order
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
}
