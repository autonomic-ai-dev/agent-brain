//! Hybrid logical clock for causal sync ordering.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hlc {
    pub wall: i64,
    pub counter: u32,
}

impl Hlc {
    pub fn zero() -> Self {
        Self {
            wall: 0,
            counter: 0,
        }
    }

    pub fn tick(self, now_wall: i64) -> Self {
        if now_wall > self.wall {
            Self {
                wall: now_wall,
                counter: 0,
            }
        } else {
            Self {
                wall: self.wall,
                counter: self.counter.saturating_add(1),
            }
        }
    }

    pub fn receive(self, other: Hlc, now_wall: i64) -> Self {
        let wall = now_wall.max(self.wall).max(other.wall);
        if wall == self.wall && wall == other.wall {
            Self {
                wall,
                counter: self.counter.max(other.counter).saturating_add(1),
            }
        } else if wall == self.wall {
            Self {
                wall,
                counter: self.counter.saturating_add(1),
            }
        } else if wall == other.wall {
            Self {
                wall,
                counter: other.counter.saturating_add(1),
            }
        } else {
            Self { wall, counter: 0 }
        }
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall
            .cmp(&other.wall)
            .then_with(|| self.counter.cmp(&other.counter))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_advances_wall_or_counter() {
        let a = Hlc {
            wall: 100,
            counter: 0,
        };
        assert_eq!(a.tick(200).wall, 200);
        assert_eq!(a.tick(50).counter, 1);
    }

    #[test]
    fn order_is_wall_then_counter() {
        let a = Hlc {
            wall: 1,
            counter: 5,
        };
        let b = Hlc {
            wall: 2,
            counter: 0,
        };
        assert!(a < b);
    }
}
