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

use flate2::read::GzDecoder;
use log::debug;
use nix::{
    errno::Errno,
    libc::off_t,
    sys::wait::waitpid,
    unistd::{chroot, fork, truncate, ForkResult},
};
use once_cell::sync::Lazy;
use std::{
    fs::{self, File},
    io,
    marker::PhantomData,
    path::{Path, PathBuf},
    process::Command,
};
use tar::Archive;
use thiserror::Error;
use uuid::Uuid;

// TODO: move this out into some sort of context struct?
// also make this configurable
static VAR_DIR_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/var/lib/fc-man"));
static IMAGE_BUILDER_DIR_PATH: Lazy<&Path> =
    Lazy::new(|| Path::new("/var/lib/fc-man/image-builder"));
static MOUNT_DIR_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/var/lib/fc-man/image-builder/mount"));

const MOUNT: &str = "mount";
const ROOTFS_FILENAME: &str = "rootfs.ext4";
const RESOLV_CONF: &str = "/etc/resolv.conf";
const MKFS_EXT4: &str = "mkfs.ext4";
const INITRAM_FS: &str = "initramfs-virt";
const VMLINUZ: &str = "vmlinuz-virt";

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

/// An image's rootfs, basically a dir that just holds all of the components we need
struct ImageRootFs<State: ImageRootFsState> {
    id: Uuid,
    working_dir: PathBuf,
    mount_dir: PathBuf,
    rootfs_file: PathBuf,
    _state: PhantomData<State>,
}

impl Default for ImageRootFs<Unmounted> {
    fn default() -> Self {
        Self::new(Uuid::new_v4())
    }
}

impl ImageRootFs<Unmounted> {
    /// Create a new root fs
    fn new(id: Uuid) -> Self {
        // TODO: change this to hash of base image fs?
        let id_str = id.to_string();

        let mut working_dir = PathBuf::from(IMAGE_BUILDER_DIR_PATH.clone());
        working_dir.push(&id_str);

        let mut rootfs_file = working_dir.clone();
        rootfs_file.push(ROOTFS_FILENAME);

        Self {
            id,
            working_dir,
            mount_dir: PathBuf::from(MOUNT_DIR_PATH.clone()),
            rootfs_file,
            _state: PhantomData,
        }
    }

    /// Sets up necessary dirs if they don't exist
    fn setup_dirs(&self) -> Result<(), ImageBuilderError> {
        let dirs: [&Path; 4] = [
            &VAR_DIR_PATH,
            &IMAGE_BUILDER_DIR_PATH,
            &self.mount_dir,
            &self.working_dir,
        ];

        for dir in dirs {
            if !Path::exists(dir) {
                debug!("Creating new dir {:?}", dir);
                fs::create_dir(dir)?;
            }
        }

        Ok(())
    }

    /// Allocate disk space for our image. This image file lives in our working dir
    fn allocate_file(&self, size: off_t) -> Result<(), ImageBuilderError> {
        debug!(
            "Allocating {} bytes to file at {:?}",
            size, &self.rootfs_file
        );
        File::create_new(&self.rootfs_file)?;
        // i think this can allocate filespace for us?
        truncate(&self.rootfs_file, size)?;
        Ok(())
    }

    /// Format our file to ext4
    fn format(&self) -> Result<(), ImageBuilderError> {
        // TODO: see if there's a better option than just shelling out to reduce implicit dependencies
        debug!("Executing command: {} {:?}", MKFS_EXT4, &self.rootfs_file);
        let output = Command::new(MKFS_EXT4).arg(&self.rootfs_file).output()?;

        // TODO: log
        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(())
    }

    /// Mounts our filesystem so we can chroot to it and change things as needed
    fn mount(self) -> Result<ImageRootFs<Mounted>, ImageBuilderError> {
        // TODO: looks like the mount syscall has different args based on linux/macos, and there's no POSIX way to
        // mount a file. I'd like to avoid conditional compilation for now, so shelling out might be the best way
        debug!(
            "Mounting image {} to {}",
            &self.rootfs_file.display(),
            &self.mount_dir.display()
        );

        let output = Command::new(MOUNT)
            .arg(&self.rootfs_file)
            .arg(&self.mount_dir)
            .output()?;

        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(ImageRootFs {
            id: self.id,
            working_dir: self.working_dir,
            mount_dir: self.mount_dir,
            rootfs_file: self.rootfs_file,
            _state: PhantomData,
        })
    }
}

impl ImageRootFs<Mounted> {
    /// Decompresses and untars our base filesystem to our mounted path
    fn copy_from_base_fs(&self, base_fs_path: &Path) -> Result<(), ImageBuilderError> {
        let compressed_tarball = File::open(base_fs_path)?;
        let tarball = GzDecoder::new(compressed_tarball);
        let mut archive = Archive::new(tarball);
        archive.unpack(&self.mount_dir)?;

        // also need to take the host's resolv.conf along so the alpine package manager works
        // TODO: clean up this unwrap
        let mount_path_str = &self.mount_dir.to_str().unwrap();
        let mounted_resolv_conf = format!("{}{}", mount_path_str, RESOLV_CONF);
        fs::copy(RESOLV_CONF, mounted_resolv_conf)?;

        Ok(())
    }

    /// Execute our final setup of the filesystem. This forks, chroots, executes the given commands
    // TODO: need to copy over resolv.conf before chroot
    fn execute_setup(&self, commands: Vec<Command>) -> Result<(), ImageBuilderError> {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                waitpid(child, None)?;
            }
            Ok(ForkResult::Child) => {
                chroot(&self.mount_dir)?;
                for mut cmd in commands {
                    cmd.status()?;
                }
            }
            // TODO: cleanup
            Err(_) => panic!("fork failed!"),
        }

        Ok(())
    }

    fn extract_vmlinuz_initramfs(&self) -> Result<(), ImageBuilderError> {
        // fs::copy(, )
        todo!()
    }

    fn unmount(&self) -> Result<(), ImageBuilderError> {
        todo!()
    }
}

/// High level image builder
#[derive(Default)]
pub struct ImageBuilder {}

impl ImageBuilder {
    pub fn build_image_from_base(
        &self,
        base_fs_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rootfs = ImageRootFs::default();
        rootfs.setup_dirs()?;
        rootfs.allocate_file(256 * 1024 * 1024)?;
        rootfs.format()?;
        rootfs.mount()?;

        Ok(())
    }
}
