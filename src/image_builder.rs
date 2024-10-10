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
    io::{self, BufReader, Read},
    marker::PhantomData,
    path::{Path, PathBuf, StripPrefixError},
    process::Command,
};
use tar::Archive;
use thiserror::Error;
use uuid::Uuid;

use crate::utils::get_alpine_setup_commands;

// TODO: move this out into some sort of context struct?
// also make this configurable
// TODO: move these to be args rather than embedded statics
static VAR_DIR_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/var/lib/fc-man"));
static IMAGE_BUILDER_DIR_PATH: Lazy<&Path> =
    Lazy::new(|| Path::new("/var/lib/fc-man/image-builder"));
static MOUNT_DIR_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/var/lib/fc-man/image-builder/mount"));
static RESOLV_CONF_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/etc/resolv.conf"));

const MOUNT: &str = "mount";
const ROOTFS_FILENAME: &str = "rootfs.ext4";
const MKFS_EXT4: &str = "mkfs.ext4";

const BOOT: &str = "boot";
const INITRAM_FS: &str = "initramfs-virt";
const VMLINUZ: &str = "vmlinuz-virt";

const GZIP_MAGIC_NUM: [u8; 3] = [0x1F, 0x8B, 0x08];

// TODO: make these not bad
#[derive(Error, Debug)]
enum ImageBuilderError {
    #[error("IO Error")]
    Io(#[from] io::Error),
    #[error("Syscall Error")]
    Syscall(#[from] Errno),
    #[error("Strip Prefix Error")]
    StripPrefix(#[from] StripPrefixError),
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
        debug!("Decompressing tarball '{}'", base_fs_path.display());
        let compressed_tarball = File::open(base_fs_path)?;
        let tarball = GzDecoder::new(compressed_tarball);
        let mut archive = Archive::new(tarball);
        debug!(
            "Copying tarball contents to '{}'",
            &self.mount_dir.display()
        );
        archive.unpack(&self.mount_dir)?;

        // also need to take the host's resolv.conf along so the alpine package manager works
        // TODO: clean up this unwrap
        let mut resolv_conf_path = self.mount_dir.clone();

        // pushing an absolute path replaces the entire existing path - so strip the leading '/' if there is one
        if RESOLV_CONF_PATH.starts_with("/") {
            resolv_conf_path.push(RESOLV_CONF_PATH.strip_prefix("/")?);
        } else {
            resolv_conf_path.push(RESOLV_CONF_PATH.clone());
        }

        debug!(
            "Copying resolv.conf from '{}' to '{}",
            RESOLV_CONF_PATH.display(),
            resolv_conf_path.display()
        );

        fs::copy(*RESOLV_CONF_PATH, resolv_conf_path)?;

        Ok(())
    }

    /// Execute our final setup of the filesystem. This forks, chroots, executes the given commands
    // TODO: need to copy over resolv.conf before chroot
    fn execute_setup(&self, commands: Vec<Command>) -> Result<(), ImageBuilderError> {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                // TODO: check this actually exits 0
                debug!("Spawned pid {}", child);
                waitpid(child, None)?;
            }
            Ok(ForkResult::Child) => {
                chroot(&self.mount_dir)?;
                for mut cmd in commands {
                    cmd.status()?;
                }
                std::process::exit(0)
            }
            // TODO: cleanup
            Err(_) => panic!("fork failed!"),
        }

        Ok(())
    }

    /// Grabs the initframfs before we unmount the rootfs and puts it in our working dir
    fn extract_initramfs(&self) -> Result<(), ImageBuilderError> {
        let mut initramfs_path = self.mount_dir.clone();
        initramfs_path.push(BOOT);
        initramfs_path.push(INITRAM_FS);

        let mut dest_path = self.working_dir.clone();
        dest_path.push(INITRAM_FS);

        debug!(
            "Copying initramfs from '{}' to '{}",
            initramfs_path.display(),
            dest_path.display()
        );

        fs::copy(initramfs_path, dest_path)?;

        Ok(())
    }

    fn find_vmlinuz_gzip_offset(&self, vmlinuz_file: &File) -> Result<(), ImageBuilderError> {
        todo!()
    }

    fn extract_and_decompress_vmlinuz(&self) -> Result<(), ImageBuilderError> {
        let mut vmlinuz_path = self.mount_dir.clone();
        vmlinuz_path.push(BOOT);
        vmlinuz_path.push(VMLINUZ);

        let vmlinuz = File::open(vmlinuz_path)?;
        let mut reader = BufReader::new(vmlinuz);
        let mut buf = [0; 1024];
        let mut gzip_magic_num_offset: usize = 0;

        loop {
            let read = reader.read(&mut buf)?;

            if read == 0 {
                break;
            }

            if let Some(offset) = buf[..read]
                .windows(GZIP_MAGIC_NUM.len())
                .position(|window| window == GZIP_MAGIC_NUM)
            {
                gzip_magic_num_offset += offset;
            }
        }

        todo!()
    }

    /// Unmounts our filesystem when we're done. This consumes self
    fn unmount(self) -> Result<(), ImageBuilderError> {
        debug!("Unmounting {}", &self.mount_dir.display());
        let output = Command::new("umount").arg(&self.mount_dir).output()?;

        if !output.stderr.is_empty() {
            debug!("{:?}", output.stderr);
        }

        Ok(())
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
        let mounted_rootfs = rootfs.mount()?;

        mounted_rootfs.copy_from_base_fs(base_fs_path)?;
        mounted_rootfs.execute_setup(get_alpine_setup_commands())?;
        mounted_rootfs.extract_initramfs()?;

        // mounted_rootfs.unmount()?;

        Ok(())
    }
}
