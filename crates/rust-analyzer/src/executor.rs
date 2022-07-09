use hir::ModuleDef;
use ide_db::base_db::Upcast;
use ide_db::runnables::{Id, RunnableDatabase, Content};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::process::{Child, Command};
use std::sync::Arc;
use ide_db::base_db::salsa::{Snapshot, ParallelDatabase};
use ide::{RootDatabase, AnalysisHost, Analysis};

pub enum ExectuinState {
    /// Indicates a test has failed, it means that the test did not
    /// finish successfully and there were problems during the execution.
    Failed,
    /// Indicates a test has errored, it means that test couldn't be
    /// executed at all, from a compilation error for example.
    Errored,
    /// Indicates a test has passed.
    Passed,
}

pub struct RunStatus {
    pub state: ExectuinState,
    pub message: String,
    pub duration: f64,
}

pub struct Executor {
    // TODO: size know at compile time point 
    analysis: Option<Arc<Mutex<AnalysisHost>>>,
    current_status: FxHashMap<Id, RunStatus>,
    executing: FxHashMap<Id, Child>,
}

impl Default for Executor {
    fn default() -> Self {
        Self {
            analysis: None,
            current_status: Default::default(),
            executing: Default::default(),
        }
    }
}

impl Executor {
    pub fn set_analysis_host(&mut self, db_resolver: Arc<Mutex<AnalysisHost>>) {
        self.analysis = Some(db_resolver);
    }

    pub fn snapshot(&self) -> Snapshot<RootDatabase> {
        self.analysis.as_ref().unwrap().lock().raw_database().snapshot()
    }
}

impl Executor {
    pub fn results(&self) -> Option<impl Iterator<Item = (&Id, &RunStatus)>> {
        if self.current_status.is_empty() {
            return None;
        }
        
        Some(self.current_status.iter())
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
                                message: todo!(),
                                duration: todo!(),
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
        unsafe {
            for id in ids {
                if self.executing.get(&id).is_some() {
                    tracing::error!(
                        "impossible run test with id: {:?}, because it is already running",
                        id
                    );
                    continue;
                }

                
                // let mut rnbl = None;
                // let workspace_rnbl = self.snapshot().workspace_runnables();
                // for crate_rnbls in workspace_rnbl.iter() {
                //     for file_rnbls in crate_rnbls.1.iter() {
                //         rnbl = file_rnbls.1.get_rnbl_by_id(id);
                //         if rnbl.is_some() {
                //             break;
                //         }
                //     }
                // }

                // if rnbl.is_none() {
                //     tracing::error!("impossible run test with id: {:?}, because it is unexist", id);
                //     continue;
                // }

                // let full_path;
                // match rnbl.unwrap() {
                //     RunnableView::Node(_) => {
                //         tracing::error!("id: {:?} corresponding to the node, but must to leaf", id);
                //         continue;
                //     }
                //     RunnableView::Leaf(leaf) => match leaf {
                //         ide_db::runnables::Runnable::Function(func) => {
                //             full_path =
                //                 ModuleDef::from(func.location).canonical_path(self.snapshot().upcast()).unwrap();
                //         }
                //         ide_db::runnables::Runnable::Doctest(_) => todo!(),
                //     },
                // }

                // // For more info read https://doc.rust-lang.org/cargo/commands/cargo-test.html
                // // Options passed to libtest https://doc.rust-lang.org/rustc/tests/index.html
                // let result = Command::new("cargo")
                //     .args([
                //         "test",
                //         full_path.as_str(),
                //         "--",
                //         "--exact",
                //         "--nocapture",
                //         "--message-format=json",
                //         "-Zunstable-options",
                //         "--report-time",
                //     ])
                //     .spawn();

                // match result {
                //     Ok(child) => {
                //         self.executing.insert(id, child);
                //     }
                //     Err(err) => {
                //         self.current_status.insert(
                //             id,
                //             RunStatus {
                //                 state: ExectuinState::Errored,
                //                 message: todo!(),
                //                 duration: todo!(),
                //             },
                //         );
                //         tracing::error!(
                //             "when trying to run test with id: {:?}, occurred error: {:?}",
                //             id,
                //             err
                //         )
                //     }
                // }
            }
        }
    }

    pub fn abort_tests(&mut self, ids: impl Iterator<Item = Id>) {
        for id in ids {
            let value = self.executing.remove(&id);
            match value {
                Some(mut child) => {
                    if let Err(err) = child.kill() {
                        tracing::error!(
                            "when trying to abort test with id: {:?}, occurred error: {:?}",
                            id,
                            err
                        )
                    }
                }
                None => tracing::error!(
                    "impossible abort test with id: {:?}, because it is'nt executing",
                    id
                ),
            }
        }
    }
}
