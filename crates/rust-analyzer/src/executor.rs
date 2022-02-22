use std::process::{Command, Child};
use hir::{ModuleDef, db::HirDatabase};
use ide_db::runnables::{Id, RunnableView, RunnableDatabase};
use rustc_hash::FxHashMap;
use ide_db::base_db::Upcast;

enum ExectuinState {
    /// Indicates a test has failed, it means that the test did not 
    /// finish successfully and there were problems during the execution.
    Failed,
    /// Indicates a test has errored, it means that test couldn't be
    /// executed at all, from a compilation error for example.
    Errored,
    /// Indicates a test has passed.
    Passed,
}

struct RunStatus {
    state: ExectuinState,
    // start: todo!(),
    // duration: todo!(),
}

pub struct Executor<'a> {
    db: &'a (impl Upcast<HirDatabase> + Upcast<HirDatabase>),
    current_status: FxHashMap<Id, RunStatus>,
    executing: FxHashMap<Id, Child>,
}

impl<'a> Executor<'a> {
    pub fn new(db: &'a (impl Upcast<HirDatabase> + Upcast<HirDatabase>)) -> Self {
        Self { 
            db, 
            current_status: Default::default(), 
            executing: Default::default() 
        }
    }

    pub fn process(&mut self) {
        self.executing.retain(|id, child| {
            let result = child.try_wait();
            match result {
                Ok(Some(status)) => {
                    match status.exit_ok() {
                        Ok(_) => {
                            self.current_status.insert(*id, RunStatus {
                                state: ExectuinState::Passed,
                            });
                            false
                        },
                        Err(err) =>{ 
                            tracing::error!("a program that was executing test with id: {:?}, finish with an error code: {:?}", id, err.code().unwrap()); 
                            false
                        },
                    }
                }
                Ok(None) => true,
                Err(err) => {
                    tracing::error!("when trying to check status of test with id: {:?}, occurred error: {:?}", id, err);
                    false
                }
            }
        });
    }
    
    pub fn run_tests(&mut self, ids: impl Iterator<Item = Id>) {
        for id in ids {
            if self.executing.get(&id).is_some() {
                tracing::error!("impossible run test with id: {:?}, because it is already running", id);
                continue;
            }

            let rnbl = self.db.upcast::<RunnableDatabase>().get_by_id(id);
            if rnbl.is_none() {
                tracing::error!("impossible run test with id: {:?}, because it is unexist", id);
                continue;
            }

            let full_path;
            match rnbl.unwrap() {
                RunnableView::Node(_) => {
                    tracing::error!("id: {:?} corresponding to the node, but must to leaf", id);
                    continue;
                },
                RunnableView::Leaf(leaf) => {
                    match leaf {
                        ide_db::runnables::Runnable::Function(func) => {
                            full_path = ModuleDef::from(func.location).canonical_path(self.db.upcast()).unwrap();
                        },
                        ide_db::runnables::Runnable::Doctest(_) => todo!(),
                    }
                },
            }

            // For more info read https://doc.rust-lang.org/cargo/commands/cargo-test.html
            // Options passed to libtest https://doc.rust-lang.org/rustc/tests/index.html
            let result = Command::new("cargo")
                .args([
                    "test", 
                    full_path.as_str(), 
                    "--", 
                    "--exact",
                    "--nocapture", 
                    "--message-format=json",
                    "-Zunstable-options",
                    "--report-time",
                ])
                .spawn();

            match result {
                Ok(child) => {
                    self.executing.insert(id, child);
                },
                Err(err) => {
                    self.current_status.insert(id, RunStatus {
                        state: ExectuinState::Errored,
                    });
                    tracing::error!("when trying to run test with id: {:?}, occurred error: {:?}", id, err)
                },
            }
        }
    }

    pub fn abort_tests(&mut self, ids: impl Iterator<Item = Id>) {
        for id in ids {
            let value = self.executing.remove(&id);
            match value {
                Some(mut child) => {
                    if let Err(err) = child.kill() {
                        tracing::error!("when trying to abort test with id: {:?}, occurred error: {:?}", id, err)
                    }
                },
                None => tracing::error!("impossible abort test with id: {:?}, because it is'nt executing", id),
            }
        }
    }
}