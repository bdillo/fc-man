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
    io::{self, BufReader, Read, Seek},
    marker::PhantomData,
    path::{Path, PathBuf, StripPrefixError},
    process::Command,
};
use tar::Archive;
use thiserror::Error;
use uuid::Uuid;

use crate::utils::get_alpine_setup_commands;

static RESOLV_CONF_PATH: Lazy<&Path> = Lazy::new(|| Path::new("/etc/resolv.conf"));

const VAR_DIR: &str = "/var/lib/fc-man";

const MOUNT: &str = "mount";
const IMAGE_BUILDER: &str = "image-builder";

const ROOTFS_FILENAME: &str = "rootfs.ext4";
const MKFS_EXT4: &str = "mkfs.ext4";

const BOOT: &str = "boot";
const INITRAM_FS: &str = "initramfs-virt";
const VMLINUZ: &str = "vmlinuz-virt";
const VMLINUX: &str = "vmlinux-virt";

const GZIP_MAGIC_NUM: [u8; 3] = [0x1F, 0x8B, 0x08];

// TODO: make these not bad
#[derive(Error, Debug)]
pub enum ImageBuilderError {
    #[error("IO Error")]
    Io(#[from] io::Error),
    #[error("Syscall Error")]
    Syscall(#[from] Errno),
    #[error("Strip Prefix Error")]
    StripPrefix(#[from] StripPrefixError),
    #[error("Unable to find GZIP header in compressed kernel file ")]
    MissingGzipHeader,
}

/// VM image with paths to all related components needed to launch a vm
pub struct Image {
    rootfs_path: PathBuf,
    initrd_path: PathBuf,
    kernel_path: PathBuf,
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
    // TODO: change this to hash of base fs
    id: String,
    working_dir: PathBuf,
    mount_dir: PathBuf,
    rootfs_file: PathBuf,
    _state: PhantomData<State>,
}

impl ImageRootFs<Unmounted> {
    /// Create a new root fs
    fn new<T>(id: &str, working_dir: T, mount_dir: T) -> Self
    where
        T: AsRef<Path>,
    {
        let working_dir = working_dir.as_ref().to_path_buf();
        let mount_dir = mount_dir.as_ref().to_path_buf();

        let mut rootfs_file = working_dir.clone();
        rootfs_file.push(ROOTFS_FILENAME);

        Self {
            id: id.to_owned(),
            working_dir,
            mount_dir,
            rootfs_file,
            _state: PhantomData,
        }
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
    fn rootfs_file(&self) -> &Path {
        &self.rootfs_file
    }

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
    fn extract_initramfs(&self) -> Result<PathBuf, ImageBuilderError> {
        // TODO: take path as arg
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

        fs::copy(&initramfs_path, &dest_path)?;

        Ok(dest_path)
    }

    fn find_vmlinuz_gzip_offset<R: Read>(&self, vmlinuz_file: R) -> Result<u64, ImageBuilderError> {
        let mut reader = BufReader::new(vmlinuz_file);
        let mut buf = [0; 1024];
        let mut gzip_magic_num_offset: usize = 0;

        loop {
            let read = reader.read(&mut buf)?;

            if read == 0 {
                // we're either done or the file is empty, either way we didn't find what we're looking for
                return Err(ImageBuilderError::MissingGzipHeader);
            } else if let Some(offset) = buf[..read]
                .windows(GZIP_MAGIC_NUM.len())
                .position(|window| window == GZIP_MAGIC_NUM)
            {
                gzip_magic_num_offset += offset;
                return Ok(gzip_magic_num_offset as u64);
            } else {
                gzip_magic_num_offset += read;
            }
        }
    }

    fn extract_and_decompress_vmlinuz(&self) -> Result<PathBuf, ImageBuilderError> {
        // TODO: take all these paths as args
        let mut vmlinuz_path = self.mount_dir.clone();
        vmlinuz_path.push(BOOT);
        vmlinuz_path.push(VMLINUZ);

        let mut vmlinuz = File::open(&vmlinuz_path)?;

        let offset = self.find_vmlinuz_gzip_offset(&vmlinuz)?;
        debug!(
            "Found gzip header at offset {} in file '{}'",
            offset,
            &vmlinuz_path.display()
        );

        vmlinuz.seek(io::SeekFrom::Start(offset))?;

        // TODO: can probably switch this to use bufreader?
        let mut gzip = GzDecoder::new(&vmlinuz);

        let mut out_path = self.working_dir.clone();
        out_path.push(VMLINUX);

        let mut out = File::create_new(&out_path)?;
        debug!("Writing decompressed kernel to '{}'", &out_path.display());
        io::copy(&mut gzip, &mut out)?;

        Ok(out_path)
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
#[derive(Debug)]
pub struct ImageBuilder {
    image_builder_dir: PathBuf,
}

impl Default for ImageBuilder {
    fn default() -> Self {
        let mut image_builder_dir = PathBuf::from(VAR_DIR);
        image_builder_dir.push(IMAGE_BUILDER);
        Self { image_builder_dir }
    }
}

impl ImageBuilder {
    fn get_working_dir(&self, id: &str) -> PathBuf {
        let mut working_dir = self.image_builder_dir.clone();
        working_dir.push(id);
        working_dir
    }

    fn get_mount_dir(&self) -> PathBuf {
        let mut mount_dir = self.image_builder_dir.clone();
        mount_dir.push(MOUNT);
        mount_dir
    }

    /// Sets up our required directories
    fn setup_dirs<T>(&self, working_dir: T, mount_dir: T) -> Result<(), ImageBuilderError>
    where
        T: AsRef<Path>,
    {
        let dirs: [&Path; 3] = [
            &self.image_builder_dir,
            working_dir.as_ref(),
            mount_dir.as_ref(),
        ];

        for dir in dirs {
            if !Path::exists(dir) {
                debug!("Creating new dir {:?}", dir);
                fs::create_dir_all(dir)?;
            }
        }

        Ok(())
    }

    pub fn build_image_from_base(&self, base_fs_path: &Path) -> Result<Image, ImageBuilderError> {
        // TODO: hash the base rootfs and use that as working dir? or is there a better way to organize this
        let id = Uuid::new_v4().to_string();

        let working_dir = self.get_working_dir(&id);
        let mount_dir = self.get_mount_dir();

        self.setup_dirs(&working_dir, &mount_dir)?;

        let rootfs = ImageRootFs::new(&id, &working_dir, &mount_dir);
        rootfs.allocate_file(256 * 1024 * 1024)?;
        rootfs.format()?;
        let mounted_rootfs = rootfs.mount()?;

        mounted_rootfs.copy_from_base_fs(base_fs_path)?;
        mounted_rootfs.execute_setup(get_alpine_setup_commands())?;

        // TODO: clean up these names to be a bit more consistent
        let initram_fs_path = mounted_rootfs.extract_initramfs()?;
        let vmlinux_path = mounted_rootfs.extract_and_decompress_vmlinuz()?;
        let rootfs_path = mounted_rootfs.rootfs_file();

        let image = Image {
            rootfs_path: rootfs_path.to_path_buf(),
            initrd_path: initram_fs_path,
            kernel_path: vmlinux_path,
        };

        mounted_rootfs.unmount()?;

        Ok(image)
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use super::*;

    fn build_image_root_fs<S>(_state: S) -> ImageRootFs<S>
    where
        S: ImageRootFsState,
    {
        ImageRootFs {
            id: "id".to_owned(),
            working_dir: PathBuf::default(),
            mount_dir: PathBuf::default(),
            rootfs_file: PathBuf::default(),
            _state: PhantomData::<S>,
        }
    }

    #[test]
    fn test_find_gzip_offset() -> Result<(), ImageBuilderError> {
        let mut successful_test_cases: Vec<(Cursor<Vec<u8>>, u64)> = vec![
            (Cursor::new(GZIP_MAGIC_NUM.to_vec()), 0),
            (
                Cursor::new({
                    let mut buf = [0x00, 0x01, 0xFF, 0x1F, 0x00].to_vec();
                    buf.extend(GZIP_MAGIC_NUM);
                    buf.extend([0xFF; 10]);
                    buf
                }),
                5,
            ),
            (
                Cursor::new({
                    let mut buf = [0xFF; 5000].to_vec();
                    buf.extend(GZIP_MAGIC_NUM);
                    buf.extend([0x00; 2048]);
                    buf
                }),
                5000,
            ),
            (
                Cursor::new({
                    let mut buf = [0x01; 100000].to_vec();
                    buf.extend(GZIP_MAGIC_NUM);
                    buf.extend([0x0F; 10]);
                    buf
                }),
                100000,
            ),
        ];

        let mounted_fs = build_image_root_fs(Mounted {});

        for (buf, expected_offset) in successful_test_cases.iter_mut() {
            let offset = mounted_fs.find_vmlinuz_gzip_offset(buf)?;
            assert_eq!(offset, *expected_offset);
        }

        let mut failed_test_cases: Vec<Cursor<&[u8]>> = vec![
            Cursor::new(&[0x00]),
            Cursor::new(&[0xFF; 1024]),
            Cursor::new(&[0x00, 0x01, 0x1F, 0x8B, 0x07, 0x01]),
        ];

        for buf in failed_test_cases.iter_mut() {
            let result = mounted_fs.find_vmlinuz_gzip_offset(buf);
            assert!(result.is_err());
        }

        Ok(())
    }
}
