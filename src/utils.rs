use std::process::Command;

const APK: &str = "/sbin/apk";
const RC_UPDATE: &str = "/sbin/rc-update";

/// Setup commands for alpine, should turn this into a config file or something
pub fn get_alpine_setup_commands() -> Vec<Command> {
    vec![
        {
            // update repos
            let mut cmd = Command::new(APK);
            cmd.args(["update"]);
            cmd
        },
        {
            // install packages
            let mut cmd = Command::new(APK);
            cmd.args([
                "add",
                "linux-vert",
                "mkinitfs", // TODO: is this needed?
                "alpine-base",
                "util-linux",
                "openrc",
                "sshd",
                "sudo",
            ]);
            cmd
        },
        {
            // setup some terminal stuff for firecracker
            let mut cmd = Command::new("/bin/ln");
            cmd.args(["-s", "agetty", "/etc/init.d/agetty.ttyS0"]);
            cmd
        },
        {
            // setup some terminal stuff for firecracker
            let mut cmd = Command::new("/bin/echo");
            // don't think this will work...
            cmd.args(["ttyS0", ">", "/etc/securetty"]);
            cmd
        },
    ]
}
