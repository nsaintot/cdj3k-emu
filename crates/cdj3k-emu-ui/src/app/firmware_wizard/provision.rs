//! Background provisioning: the pipeline a run walks through on its worker
//! thread, reporting its stage and log to whoever watches it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::Context;

use cdj3k_emu_firmware::LuksKey;
use cdj3k_emu_panel::Model;
use cdj3k_emu_storage::{FirmwareInfo, StagedFirmware};

#[derive(Clone, Debug)]
pub(super) enum ProvisionStep {
    Decrypting,
    ExtractingKernel,
    PatchingInitramfs,
    CreatingEmmc,
    Done,
    Error(String),
}

/// The four stages a run goes through, in the order the meter draws them.
pub(super) const STAGES: [&str; 4] = [
    "Decrypt",
    "Extract kernel",
    "Patch initramfs",
    "Create eMMC image",
];

impl ProvisionStep {
    pub(super) fn label(&self) -> &str {
        match self {
            Self::Decrypting => "Decrypting firmware…",
            Self::ExtractingKernel => "Extracting kernel…",
            Self::PatchingInitramfs => "Patching initramfs…",
            Self::CreatingEmmc => "Creating eMMC image…",
            Self::Done => "Done",
            Self::Error(_) => "Error",
        }
    }

    /// Which of [`STAGES`] the run is on, if it is still on one.
    pub(super) fn stage(&self) -> Option<usize> {
        match self {
            Self::Decrypting => Some(0),
            Self::ExtractingKernel => Some(1),
            Self::PatchingInitramfs => Some(2),
            Self::CreatingEmmc => Some(3),
            Self::Done | Self::Error(_) => None,
        }
    }

    pub(super) fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error(_))
    }
}

/// Why an update file cannot go in a slot being set up as `model`, if it
/// cannot.
///
/// A `.UPD` is a bare LUKS container whose header names no product, so only
/// its file name identifies the deck: a CDJ-3000 slot takes a `CDJ3K…`, a
/// CDJ-3000X slot a `CDJ3000X…`, and any other name is refused.
pub(super) fn upd_name_objection(path: &str, model: Model, slot: u32) -> Option<String> {
    match Model::from_firmware_file_name(path) {
        Some(named) if named == model => None,
        Some(named) => Some(format!(
            "This is a {named} update. Slot {slot} is being set up as a {model}."
        )),
        None => Some(format!(
            "This is not a {model} update. Its name should be {}, as Pioneer publishes it.",
            name_list(model)
        )),
    }
}

/// "CDJ3K… or CDJ3000…", the names a deck's update file goes by.
fn name_list(model: Model) -> String {
    let names: Vec<String> = model
        .firmware_file_names()
        .iter()
        .map(|n| format!("{n}…"))
        .collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

fn bundled_resources() -> std::path::PathBuf {
    cdj3k_emu_platform::bundled::resources()
}

/// One line of a run's log, formatted in place.
macro_rules! log {
    ($run:expr, $($arg:tt)*) => {
        $run.log(&format!($($arg)*))
    };
}

/// Where a run reports: its stage, its log, and the window to repaint as
/// either changes.
struct Run {
    status: Arc<Mutex<ProvisionStep>>,
    log: Arc<Mutex<String>>,
    ctx: Context,
}

/// The run has stopped; its status says why.
struct Stopped;

impl Run {
    fn set(&self, step: ProvisionStep) {
        *self.status.lock().unwrap() = step;
        self.ctx.request_repaint();
    }

    fn log(&self, line: &str) {
        eprintln!("cdj3k-emu-provision: {line}");
        {
            let mut g = self.log.lock().unwrap();
            g.push_str(line);
            g.push('\n');
        }
        self.ctx.request_repaint();
    }

    /// A step's value, or the run stopped with its error in the log.
    fn check<T, E: ToString>(&self, result: Result<T, E>) -> Result<T, Stopped> {
        result.map_err(|e| {
            log!(self, "[error] {}", e.to_string());
            self.set(ProvisionStep::Error(
                "Error while processing: see the console for details".to_string(),
            ));
            Stopped
        })
    }
}

/// What a run installs, and where.
struct Job {
    upd_path: PathBuf,
    key: Option<LuksKey>,
    instance: u32,
    model: Model,
    ssh: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn provision(
    upd_path: PathBuf,
    key: Option<LuksKey>,
    target_instance: u32,
    model: Model,
    ssh: bool,
    status: Arc<Mutex<ProvisionStep>>,
    log: Arc<Mutex<String>>,
    ctx: Context,
) {
    let run = Run { status, log, ctx };
    let job = Job {
        upd_path,
        key,
        instance: target_instance,
        model,
        ssh,
    };
    if install(&job, &run).is_ok() {
        log!(run, "[done] all steps completed");
        run.set(ProvisionStep::Done);
    }
}

fn install(job: &Job, run: &Run) -> Result<(), Stopped> {
    log!(
        run,
        "[slot] target = instance-{} ({})",
        job.instance,
        job.model
    );
    let staged = run.check(StagedFirmware::new(job.instance))?;
    let paths = staged.paths.clone();
    log!(run, "[slot] staging dir = {}", paths.dir().display());

    // An `.iso` has been unwrapped by hand already and is taken on trust.
    if job.key.is_some() {
        check_name(job, run)?;
    }

    // 1. Decrypt UPD → ISO in the staging dir (or use the given ISO as is)
    run.set(ProvisionStep::Decrypting);
    let tmp_iso = decrypt(job, paths.dir(), run)?;
    let firmware_info = read_firmware_info(&tmp_iso, run);

    // 3. Install vanilla kernel + extract Pioneer initramfs
    run.set(ProvisionStep::ExtractingKernel);
    let out_dir = paths.dir().to_path_buf();
    run.check(std::fs::create_dir_all(&out_dir))?;
    // The vanilla kernel ships in the app bundle.
    let resources_dir = bundled_resources();
    let kernel_src = resources_dir.join("Image");
    install_kernel(&kernel_src, &paths.kernel, run)?;

    // 4. Patch initramfs - Pioneer firmware is the rootfs base
    //    (the app binary and Pioneer libraries live there).
    run.set(ProvisionStep::PatchingInitramfs);
    log!(run, "[patch] resources dir: {}", resources_dir.display());
    let initramfs_orig = out_dir.join("initramfs-orig.cpio");
    extract_initramfs(&tmp_iso, &out_dir, &initramfs_orig, run)?;
    let patch = Patch {
        orig: &initramfs_orig,
        resources: &resources_dir,
        kernel: &kernel_src,
        out: &paths.initramfs,
    };
    patch_initramfs(&patch, job.ssh, run)?;
    log!(run, "[patch] OK → {}", paths.initramfs.display());
    let _ = std::fs::remove_file(&initramfs_orig);

    // 5. Create the eMMC qcow2
    run.set(ProvisionStep::CreatingEmmc);
    let serial = soc_serial(run)?;
    log!(run, "[serial] SoC serial = {}", serial);
    let cabinet = stage_cabinet(&tmp_iso, job.model, &serial, run);
    let release = firmware_info.release.clone();
    create_emmc(job, paths.emmc.clone(), firmware_info, cabinet, run)?;

    // 6. Record the staged installation as complete, for the slot's owner to
    //    swap in.
    if let Some(prev) = cdj3k_emu_storage::InstanceSettings::saved_model(job.instance) {
        if prev != job.model {
            log!(run, "[slot] replaces the {prev} installation once applied");
        }
    }
    let _ = std::fs::remove_file(paths.dir().join("firmware.iso.tmp"));
    run.check(staged.finish(job.model, release.as_deref(), &serial))
}

/// Refuse a `.UPD` whose name is not one of the model's.
fn check_name(job: &Job, run: &Run) -> Result<(), Stopped> {
    let name = job.upd_path.to_string_lossy();
    match upd_name_objection(&name, job.model, job.instance) {
        Some(objection) => {
            log!(run, "[deck] {objection}");
            run.set(ProvisionStep::Error(objection));
            Err(Stopped)
        }
        None => {
            log!(run, "[deck] named as a {} update", job.model);
            Ok(())
        }
    }
}

/// The firmware's ISO image: the `.UPD` decrypted into `dir`, or the given
/// file when it is one already.
fn decrypt(job: &Job, dir: &Path, run: &Run) -> Result<PathBuf, Stopped> {
    let Some(key) = &job.key else {
        log!(
            run,
            "[decrypt] skipped - {} is already an ISO 9660 image",
            job.upd_path.display()
        );
        return Ok(job.upd_path.clone());
    };
    log!(run, "[decrypt] opening {}", job.upd_path.display());
    let tmp_iso = dir.join("firmware.iso.tmp");
    log!(run, "[decrypt] output → {}", tmp_iso.display());
    run.check(cdj3k_emu_firmware::decrypt_upd(
        &job.upd_path,
        key,
        &tmp_iso,
    ))?;
    log!(run, "[decrypt] OK - ISO written");
    Ok(tmp_iso)
}

fn read_firmware_info(iso: &Path, run: &Run) -> FirmwareInfo {
    match cdj3k_emu_firmware::read_firmware_info(iso) {
        Ok(info) => {
            let unknown = "(unknown)";
            log!(
                run,
                "[firmware] release={} rev_apl={} rev_system={} miniloader={}",
                info.release.as_deref().unwrap_or(unknown),
                info.rev_apl.as_deref().unwrap_or(unknown),
                info.rev_system.as_deref().unwrap_or(unknown),
                info.miniloader.as_deref().unwrap_or(unknown),
            );
            info
        }
        Err(e) => {
            log!(
                run,
                "[firmware] could not read version info ({e}), defaulting to all-unknown"
            );
            FirmwareInfo::default()
        }
    }
}

fn install_kernel(kernel_src: &Path, kernel_out: &Path, run: &Run) -> Result<(), Stopped> {
    log!(
        run,
        "[kernel] installing kernel {} → {}",
        kernel_src.display(),
        kernel_out.display()
    );
    run.check(
        std::fs::copy(kernel_src, kernel_out)
            .map(|_| ())
            .map_err(|e| std::io::Error::other(format!("copy Image: {e}"))),
    )?;
    log!(run, "[kernel] OK");
    Ok(())
}

/// The Pioneer initramfs, unpacked to `initramfs_orig`. A model with
/// images/boot.img ships it as that image's ramdisk; the others embed it in
/// their kernel, extracted to a temp file only to unpack it (not used as the
/// final kernel).
fn extract_initramfs(
    iso: &Path,
    out_dir: &Path,
    initramfs_orig: &Path,
    run: &Run,
) -> Result<(), Stopped> {
    let from_boot_img = run.check(
        cdj3k_emu_firmware::extract_boot_ramdisk(iso, initramfs_orig)
            .map_err(|e| std::io::Error::other(format!("{e}"))),
    )?;
    if from_boot_img {
        log!(
            run,
            "[initramfs] boot.img ramdisk → {}",
            initramfs_orig.display()
        );
    } else {
        let pioneer_kernel_tmp = out_dir.join("Image.pioneer.tmp");
        log!(
            run,
            "[initramfs] extracting Pioneer kernel (initramfs source) → {}",
            pioneer_kernel_tmp.display()
        );
        run.check(
            cdj3k_emu_firmware::extract_kernel(iso, &pioneer_kernel_tmp, |msg| {
                log!(run, "{}", msg)
            })
            .map_err(|e| std::io::Error::other(format!("{e}"))),
        )?;
        log!(
            run,
            "[initramfs] extracting from Pioneer kernel → {}",
            initramfs_orig.display()
        );
        run.check(
            cdj3k_emu_firmware::extract_initramfs(&pioneer_kernel_tmp, initramfs_orig)
                .map_err(|e| std::io::Error::other(format!("{e:?}"))),
        )?;
        let _ = std::fs::remove_file(&pioneer_kernel_tmp);
    }
    log!(run, "[initramfs] OK");
    Ok(())
}

/// The files an initramfs patch reads and writes.
struct Patch<'a> {
    orig: &'a Path,
    resources: &'a Path,
    /// The kernel the provisioning guest boots.
    kernel: &'a Path,
    out: &'a Path,
}

/// Patching happens inside a throwaway guest, where the scripts' modes,
/// symlinks and setuid bits hold. Setting `CDJ3K_INITRAMFS_HOST` patches on
/// the host instead, which needs bash, GNU sed and cpio and drops setuid.
fn patch_initramfs(patch: &Patch<'_>, ssh: bool, run: &Run) -> Result<(), Stopped> {
    if std::env::var_os("CDJ3K_INITRAMFS_HOST").is_some() {
        run.check(
            cdj3k_emu_firmware::patch_initramfs(patch.orig, patch.resources, patch.out)
                .map_err(|e| std::io::Error::other(e.to_string())),
        )?;
        return Ok(());
    }
    let scratch = patch.out.with_extension("handback.img");
    let size = run.check(
        cdj3k_emu_firmware::initramfs_guest::scratch_size_for(patch.orig)
            .map_err(|e| std::io::Error::other(e.to_string())),
    )?;
    run.check(std::fs::File::create(&scratch).and_then(|f| f.set_len(size)))?;
    log!(
        run,
        "[patch] provisioning guest: {} MiB scratch",
        size / (1 << 20)
    );

    let runner = cdj3k_emu_runtime::QemuGuestRunner {
        ssh,
        ..Default::default()
    };
    if ssh {
        log!(run, "[patch] root SSH enabled");
    }
    let result = cdj3k_emu_firmware::patch_initramfs_in_guest(
        patch.orig,
        patch.resources,
        patch.out,
        &cdj3k_emu_firmware::GuestProvision {
            kernel: patch.kernel,
            scratch: &scratch,
            runner: &runner,
        },
    );
    let _ = std::fs::remove_file(&scratch);
    run.check(result.map_err(|e| std::io::Error::other(e.to_string())))?;
    Ok(())
}

/// The eMMC is new, so the slot needs the serial `genkey_pr` hashes into the
/// cabinet passphrase - which the cabinet is keyed for in [`stage_cabinet`].
/// It is recorded with the install. CDJ3K_SOC_SERIAL pins a particular
/// deck's, which is what a cabinet lifted off that deck already opens with.
fn soc_serial(run: &Run) -> Result<String, Stopped> {
    match std::env::var("CDJ3K_SOC_SERIAL") {
        Ok(pinned) => run.check(cdj3k_emu_storage::InstanceSettings::parse_soc_serial(
            &pinned,
        )),
        Err(_) => Ok(cdj3k_emu_storage::InstanceSettings::mint_soc_serial()),
    }
}

/// cabinet.img carries the Widevine keys and the Device Library Plus key
/// file.  The deck's updater copies it onto the settings partition, where
/// apl_start.sh opens it on every boot (`genkey_pr | initoptenv`); here it
/// is staged in a partition the emulator does not boot from, for guest
/// patch 14.
/// The firmware's own image is keyed for the factory passphrase genkey_pb
/// computes (model + images.tar.gz, or app.hash); a keyslot for this slot's
/// own serial is added to it.
fn stage_cabinet(iso: &Path, model: Model, serial: &str, run: &Run) -> Option<Vec<u8>> {
    let mut cabinet = read_cabinet(iso, run);
    if let Some(bytes) = cabinet.as_mut() {
        key_cabinet(bytes, iso, model, serial, run);
    }
    match &cabinet {
        Some(b) => log!(run, "[cabinet] staging {} MiB", b.len() / 1_048_576),
        None => log!(run, "[cabinet] nothing to stage"),
    }
    cabinet
}

/// The firmware's cabinet.img, or the one CDJ3K_CABINET_IMG names - a cabinet
/// taken off a real deck.
fn read_cabinet(iso: &Path, run: &Run) -> Option<Vec<u8>> {
    let Some(path) = std::env::var_os("CDJ3K_CABINET_IMG") else {
        return cdj3k_emu_firmware::read_cabinet_image(iso);
    };
    let path = std::path::PathBuf::from(path);
    match std::fs::read(&path) {
        Ok(bytes) if bytes.starts_with(b"LUKS\xba\xbe") => {
            log!(
                run,
                "[cabinet] {} ({} MiB)",
                path.display(),
                bytes.len() / 1_048_576
            );
            Some(bytes)
        }
        Ok(_) => {
            log!(
                run,
                "[cabinet] {} is not a LUKS container - skipped",
                path.display()
            );
            None
        }
        Err(e) => {
            log!(run, "[cabinet] {}: {e} - skipped", path.display());
            None
        }
    }
}

/// Give the container a keyslot this slot can open.  The firmware ships it
/// keyed for the factory passphrase (genkey_pb, from the model and this
/// ISO's images.tar.gz or app.hash); opening that slot recovers the master
/// key every cabinet of the model shares, and a slot keyed for this unit's
/// serial is added from it.  A container no factory slot opens - a foreign
/// cabinet, or one from another firmware version - is refused and stages
/// unchanged.
fn key_cabinet(bytes: &mut [u8], iso: &Path, model: Model, serial: &str, run: &Run) {
    let model_env = model.spec().model_env;
    let vendor = cdj3k_emu_firmware::read_cabinet_seed(iso)
        .map(|seed| cdj3k_emu_firmware::vendor_passphrase(model_env, &seed));
    match (
        vendor,
        cdj3k_emu_firmware::cabinet_passphrase(model_env, serial),
    ) {
        (Some(vendor), Ok(unit)) => match cdj3k_emu_firmware::add_keyslot(bytes, &vendor, &unit) {
            Ok(slot) => log!(run, "[cabinet] keyed for {serial} in slot {slot}"),
            Err(e) => log!(
                run,
                "[cabinet] not keyed for this slot ({e}) - staging it as it is"
            ),
        },
        (None, _) => {
            log!(
                run,
                "[cabinet] no images.tar.gz or app.hash to key from - staging it as it is"
            )
        }
        (_, Err(e)) => {
            log!(
                run,
                "[cabinet] no passphrase for this slot ({e}) - staging it as it is"
            )
        }
    }
}

fn create_emmc(
    job: &Job,
    path: PathBuf,
    firmware: FirmwareInfo,
    cabinet: Option<Vec<u8>>,
    run: &Run,
) -> Result<(), Stopped> {
    log!(run, "[emmc] provisioning → {}", path.display());
    let cfg = cdj3k_emu_storage::EmmcConfig {
        path,
        instance_id: job.instance,
        model: job.model,
        firmware,
        cabinet,
    };
    run.check(cdj3k_emu_storage::provision_emmc(&cfg))?;
    log!(run, "[emmc] OK");
    Ok(())
}
