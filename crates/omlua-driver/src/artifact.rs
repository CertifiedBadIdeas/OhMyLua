use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);

pub struct LuaArtifact {
    final_path: PathBuf,
    temporary_path: PathBuf,
    _lock_file: fs::File,
}

impl LuaArtifact {
    pub fn prepare(project_dir: &Path) -> io::Result<Self> {
        let output_directory = project_dir.join("target").join("omlua");
        fs::create_dir_all(&output_directory)
            .map_err(|error| path_error("create Lua output directory", &output_directory, error))?;
        let lock_path = output_directory.join(".build.lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| path_error("open Lua build lock", &lock_path, error))?;
        lock_file
            .lock()
            .map_err(|error| path_error("lock Lua build", &lock_path, error))?;
        let final_path = output_directory.join("program.lua");
        match fs::remove_file(&final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(path_error("remove stale Lua artifact", &final_path, error));
            }
        }
        let sequence = NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed);
        let temporary_path = output_directory.join(format!(
            ".program.lua.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        Ok(Self {
            final_path,
            temporary_path,
            _lock_file: lock_file,
        })
    }

    pub fn commit(self, source: &str) -> io::Result<PathBuf> {
        let directory = self
            .final_path
            .parent()
            .expect("Lua artifact path always has a parent");
        fs::create_dir_all(directory)
            .map_err(|error| path_error("create Lua output directory", directory, error))?;

        let result = self.write_and_rename(source);
        if let Err(error) = result {
            return match fs::remove_file(&self.temporary_path) {
                Ok(()) => Err(error),
                Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => Err(error),
                Err(cleanup) => Err(io::Error::new(
                    cleanup.kind(),
                    format!(
                        "{error}; additionally failed to remove temporary Lua artifact `{}`: {cleanup}",
                        self.temporary_path.display()
                    ),
                )),
            };
        }
        Ok(self.final_path)
    }

    fn write_and_rename(&self, source: &str) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.temporary_path)
            .map_err(|error| {
                path_error("create temporary Lua artifact", &self.temporary_path, error)
            })?;
        file.write_all(source.as_bytes()).map_err(|error| {
            path_error("write temporary Lua artifact", &self.temporary_path, error)
        })?;
        file.sync_all().map_err(|error| {
            path_error(
                "synchronize temporary Lua artifact",
                &self.temporary_path,
                error,
            )
        })?;
        drop(file);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|error| {
            path_error(
                &format!(
                    "rename temporary Lua artifact `{}` to",
                    self.temporary_path.display()
                ),
                &self.final_path,
                error,
            )
        })
    }
}

fn path_error(action: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("failed to {action} `{}`: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("omlua-artifact-test-{}-{name}", std::process::id()))
    }

    fn reset(path: &Path) {
        match fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to reset test directory: {error}"),
        }
        fs::create_dir(path).unwrap();
    }

    #[test]
    fn prepare_removes_only_the_previous_final_artifact() {
        let directory = directory("prepare");
        reset(&directory);
        let output = directory.join("target/omlua");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("program.lua"), "stale").unwrap();
        fs::write(output.join("keep.txt"), "keep").unwrap();

        let artifact = LuaArtifact::prepare(&directory).unwrap();
        assert!(!output.join("program.lua").exists());
        assert_eq!(fs::read_to_string(output.join("keep.txt")).unwrap(), "keep");
        assert!(!artifact.temporary_path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn commit_writes_an_adjacent_temporary_file_and_renames_it() {
        let directory = directory("commit");
        reset(&directory);
        let artifact = LuaArtifact::prepare(&directory).unwrap();
        let temporary = artifact.temporary_path.clone();
        assert_eq!(temporary.parent(), artifact.final_path.parent());
        assert!(
            temporary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&std::process::id().to_string())
        );

        let final_path = artifact.commit("return 42\n").unwrap();
        assert_eq!(fs::read_to_string(final_path).unwrap(), "return 42\n");
        assert!(!temporary.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_directory_failure_leaves_no_final_artifact() {
        let directory = directory("failure");
        reset(&directory);
        fs::create_dir(directory.join("target")).unwrap();
        fs::write(directory.join("target/omlua"), "not a directory").unwrap();
        let error = LuaArtifact::prepare(&directory).err().unwrap();
        assert!(error.to_string().contains("create Lua output directory"));
        assert!(!directory.join("target/omlua/program.lua").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_builds_are_serialized_for_the_whole_artifact_lifetime() {
        use std::sync::{Arc, Barrier, mpsc};
        use std::thread;
        use std::time::Duration;

        let directory = directory("locking");
        reset(&directory);
        let first = LuaArtifact::prepare(&directory).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let thread_barrier = Arc::clone(&barrier);
        let thread_directory = directory.clone();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            thread_barrier.wait();
            sender
                .send(LuaArtifact::prepare(&thread_directory))
                .unwrap();
        });

        barrier.wait();
        thread::sleep(Duration::from_millis(50));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(first);
        let second = receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        drop(second);
        handle.join().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
