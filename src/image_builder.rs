// make fs
// format fs
// mount fs
// copy from image
// chroot
// customize rootfs, openssh, openrc, etc
// grab initramfs, vmlinuz
// extract vmlinux from vmlinuz
// unmount fs
// done!

use log::debug;
use nix::{errno::Errno, libc::off_t, mount::mount, unistd::truncate};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

// TODO: move this out into some sort of context struct?
const VAR_DIR: &str = "/var/lib/fc-man";
const ROOTFS_DIR: &str = "rootfs";
const MOUNT: &str = "mount";

const MKFS_EXT4: &str = "mkfs.ext4";

// TODO: make these not bad
#[derive(Error, Debug)]
enum ImageBuilderError {
    #[error("IO Error")]
    IoError(#[from] io::Error),
    #[error("Syscall error")]
    SyscallError(#[from] Errno),
}

/// VM image with paths to all related components needed to launch a vm
struct Image<'a> {
    rootfs_path: &'a Path,
    initrd_path: &'a Path,
    kernel_path: &'a Path,
}

/// Marker trait for our filesystem state structs. Doing this to restrict what types `ImageRootFs` is generic over
pub trait ImageRootFsState {}

/// The states of our image root filesystems - either mounted or unmounted
struct Unmounted;
struct Mounted;

impl ImageRootFsState for Unmounted {}
impl ImageRootFsState for Mounted {}

/// An image's rootfs
struct ImageRootFs<State: ImageRootFsState> {
    name: String,
    path: PathBuf,
    state: State,
    // raw_fd: RawFd,
}

impl ImageRootFs<Unmounted> {
    /// Create a new root fs with given name - this creates the file
    fn new(image_name: &str) -> Result<Self, ImageBuilderError> {
        let path = PathBuf::from(format!("{VAR_DIR}/{ROOTFS_DIR}/{image_name}"));
        debug!("Creating new image root fs at {:?}", &path);
        // let file = File::create_new(&path)?;
        // let raw_fd = file.as_raw_fd();

        // Ok(Self { path, raw_fd })
        Ok(Self {
            name: image_name.to_owned(),
            path,
            state: Unmounted,
        })
    }

    /// Allocate disk space for our image
    fn allocate_file(&self, size: off_t) -> Result<(), ImageBuilderError> {
        debug!("Allocating {} bytes to file at {:?}", size, &self.path);
        Ok(truncate(&self.path, size)?)
    }

    /// Format our file to ext4
    fn format(&self) -> Result<(), ImageBuilderError> {
        // TODO: see if there's a better option than just shelling out to reduce implicit dependencies
        debug!("Executing command: {} {:?}", MKFS_EXT4, &self.path);
        let output = Command::new(MKFS_EXT4).arg(&self.path).output()?;

        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(())
    }

    fn mount(self) -> Result<ImageRootFs<Mounted>, ImageBuilderError> {
        let mount_dir_name = format!("{}/{}/{}", VAR_DIR, MOUNT, &self.name);
        let mount_dir = Path::new(&mount_dir_name);
        fs::create_dir(mount_dir)?;

        // TODO: looks like the mount syscall has different args based on linux/macos, don't really want to do
        // conditional compilation. Should find a better solution
        let output = Command::new(MOUNT)
            .arg(&self.path)
            .arg(mount_dir)
            .output()?;

        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(ImageRootFs {
            name: self.name,
            path: self.path,
            state: Mounted,
        })
    }
}

impl ImageRootFs<Mounted> {
    fn copy_from_base_fs(&self, base_fs_path: &Path) -> Result<(), ImageBuilderError> {
        todo!()
    }
}

/// High level image builder
struct ImageBuilder<'a> {
    working_dir: &'a Path,
}

impl<'a> ImageBuilder<'a> {
    fn new() -> Self {
        Self {
            working_dir: Path::new(VAR_DIR),
        }
    }

    // fn copy_base_filesystem(&self, base_fs_path: &Path) -> Result<(), ImageBuilderError> {
    //     todo!()
    // }

    fn chroot(&self, image_root_fs: &ImageRootFs<Mounted>) -> Result<(), ImageBuilderError> {
        todo!()
    }
}
