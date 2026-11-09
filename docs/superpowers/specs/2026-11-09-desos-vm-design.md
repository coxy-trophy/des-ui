# DesOS disposable VM prototype

The owner instructed proceeding with a DesOS VM prototype after desktop
stabilization and disposable startup recovery. This is an experimental guest,
with scripts and images retained privately under ignored `target/desos-vm/`.
It is not a distributable ISO or a host installation.

Use Ubuntu 26.04 LTS amd64 user space, matching the project's documented package
target. Start with the Ubuntu kernel to establish a comparison baseline; an
unmodified kernel.org kernel is the subsequent experiment. Preserve Ubuntu's
kernel and a known-working guest snapshot for recovery.

## Approach

A customized installer/live ISO would add image assembly and installation work
before testing target-library compatibility. A source-built distribution would
also add package and system maintenance. For this prototype, use Canonical's
signed cloud image, a private writable qcow2 layer and cloud-init provisioning.
This is the implementation choice for the owner's authorized VM prototype.

Run QEMU/KVM with a virtual GPU, 4 virtual CPUs, approximately 5 GiB RAM, a 2 GiB
guest swap reserve and a sparse guest disk capped at 48 GiB. Bind SSH to loopback; keep QMP and any display
control sockets private. Provision an independent test account with a dedicated
SSH key; never copy host account secrets, application state or personal files.
Source transfer includes only frozen tracked source at `d462986`.

## Scope and acceptance

1. Verify the downloaded image using Canonical's published signing fingerprint,
   signed SHA256SUMS and the actual image hash. Boot using a writable overlay.
2. Confirm Ubuntu release, kernel, cloud-init completion and SSH access. Retain
   serial output and a base recovery checkpoint.
3. Build DEs-UI from the frozen source inside Ubuntu, retaining exact toolchain,
   package/library versions and build output. Do not reuse Kali-linked binaries.
4. Install only inside the guest, start DEs-UI on its virtual DRM device, and
   verify initialization, process identities and a rendered desktop. Record
   observed failures and any remaining login/recovery cases honestly.

No host kernel, bootloader, display-manager, user service or container changes.
No published packages, release version changes, physical device passthrough or
claims of laptop compatibility from virtual hardware. Use snapshots before
disruptive guest experiments. Keep guest credentials outside Git and logs.
