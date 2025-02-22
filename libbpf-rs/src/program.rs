use std::ffi::c_void;
use std::ffi::CStr;
use std::mem;
use std::mem::size_of;
use std::mem::size_of_val;
use std::os::unix::io::AsFd;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::BorrowedFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::io::OwnedFd;
use std::path::Path;
use std::ptr;
use std::ptr::NonNull;
use std::slice;

use libbpf_sys::bpf_func_id;
use num_enum::TryFromPrimitive;
use strum_macros::Display;

use crate::util;
use crate::AsRawLibbpf;
use crate::Error;
use crate::Link;
use crate::Result;

/// Options to optionally be provided when attaching to a uprobe.
#[derive(Clone, Debug, Default)]
pub struct UprobeOpts {
    /// Offset of kernel reference counted USDT semaphore.
    pub ref_ctr_offset: usize,
    /// Custom user-provided value accessible through `bpf_get_attach_cookie`.
    pub cookie: u64,
    /// uprobe is return probe, invoked at function return time.
    pub retprobe: bool,
    /// Function name to attach to.
    ///
    /// Could be an unqualified ("abc") or library-qualified "abc@LIBXYZ" name.
    /// To specify function entry, `func_name` should be set while `func_offset`
    /// argument to should be 0. To trace an offset within a function, specify
    /// `func_name` and use `func_offset` argument to specify offset within the
    /// function. Shared library functions must specify the shared library
    /// binary_path.
    pub func_name: String,
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

/// Options to optionally be provided when attaching to a USDT.
#[derive(Clone, Debug, Default)]
pub struct UsdtOpts {
    /// Custom user-provided value accessible through `bpf_usdt_cookie`.
    pub cookie: u64,
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

impl From<UsdtOpts> for libbpf_sys::bpf_usdt_opts {
    fn from(opts: UsdtOpts) -> Self {
        let UsdtOpts {
            cookie,
            _non_exhaustive,
        } = opts;
        libbpf_sys::bpf_usdt_opts {
            sz: size_of::<Self>() as _,
            usdt_cookie: cookie,
        }
    }
}

/// Options to optionally be provided when attaching to a tracepoint.
#[derive(Clone, Debug, Default)]
pub struct TracepointOpts {
    /// Custom user-provided value accessible through `bpf_get_attach_cookie`.
    pub cookie: u64,
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

impl From<TracepointOpts> for libbpf_sys::bpf_tracepoint_opts {
    fn from(opts: TracepointOpts) -> Self {
        let TracepointOpts {
            cookie,
            _non_exhaustive,
        } = opts;

        libbpf_sys::bpf_tracepoint_opts {
            sz: size_of::<Self>() as _,
            bpf_cookie: cookie,
        }
    }
}

/// Represents a parsed but not yet loaded BPF program.
///
/// This object exposes operations that need to happen before the program is loaded.
#[derive(Debug)]
pub struct OpenProgram {
    ptr: NonNull<libbpf_sys::bpf_program>,
    section: String,
}

// TODO: Document variants.
#[allow(missing_docs)]
impl OpenProgram {
    pub(crate) unsafe fn new(ptr: NonNull<libbpf_sys::bpf_program>) -> Result<Self> {
        // Get the program section
        // SAFETY:
        // bpf_program__section_name never returns NULL, so no need to check the pointer.
        let section = unsafe { libbpf_sys::bpf_program__section_name(ptr.as_ptr()) };
        let section = util::c_ptr_to_string(section)?;

        Ok(Self { ptr, section })
    }

    pub fn set_prog_type(&mut self, prog_type: ProgramType) {
        unsafe {
            libbpf_sys::bpf_program__set_type(self.ptr.as_ptr(), prog_type as u32);
        }
    }

    // The `ProgramType` of this `OpenProgram`.
    pub fn prog_type(&self) -> ProgramType {
        match ProgramType::try_from(unsafe { libbpf_sys::bpf_program__type(self.ptr.as_ptr()) }) {
            Ok(ty) => ty,
            Err(_) => ProgramType::Unknown,
        }
    }

    pub fn set_attach_type(&mut self, attach_type: ProgramAttachType) {
        unsafe {
            libbpf_sys::bpf_program__set_expected_attach_type(
                self.ptr.as_ptr(),
                attach_type as u32,
            );
        }
    }

    pub fn set_ifindex(&mut self, idx: u32) {
        unsafe {
            libbpf_sys::bpf_program__set_ifindex(self.ptr.as_ptr(), idx);
        }
    }

    /// Set the log level for the bpf program.
    ///
    /// The log level is interpreted by bpf kernel code and interpretation may
    /// change with newer kernel versions. Refer to the kernel source code for
    /// details.
    ///
    /// In general, a value of `0` disables logging while values `> 0` enables
    /// it.
    pub fn set_log_level(&mut self, log_level: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_program__set_log_level(self.ptr.as_ptr(), log_level) };
        util::parse_ret(ret)
    }

    /// Name of the section this `OpenProgram` belongs to.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// The name of this `OpenProgram`.
    pub fn name(&self) -> Result<&str> {
        let name_ptr = unsafe { libbpf_sys::bpf_program__name(self.ptr.as_ptr()) };
        let name_c_str = unsafe { CStr::from_ptr(name_ptr) };
        name_c_str.to_str().map_err(Error::with_invalid_data)
    }

    /// Set whether a bpf program should be automatically loaded by default
    /// when the bpf object is loaded.
    pub fn set_autoload(&mut self, autoload: bool) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_program__set_autoload(self.ptr.as_ptr(), autoload) };
        util::parse_ret(ret)
    }

    pub fn set_attach_target(
        &mut self,
        attach_prog_fd: i32,
        attach_func_name: Option<String>,
    ) -> Result<()> {
        let ret = if let Some(name) = attach_func_name {
            // NB: we must hold onto a CString otherwise our pointer dangles
            let name_c = util::str_to_cstring(&name)?;
            unsafe {
                libbpf_sys::bpf_program__set_attach_target(
                    self.ptr.as_ptr(),
                    attach_prog_fd,
                    name_c.as_ptr(),
                )
            }
        } else {
            unsafe {
                libbpf_sys::bpf_program__set_attach_target(
                    self.ptr.as_ptr(),
                    attach_prog_fd,
                    ptr::null(),
                )
            }
        };
        util::parse_ret(ret)
    }

    pub fn set_flags(&self, flags: u32) -> Result<()> {
        let ret = unsafe { libbpf_sys::bpf_program__set_flags(self.ptr.as_ptr(), flags) };
        util::parse_ret(ret)
    }

    /// Returns the number of instructions that form the program.
    ///
    /// Note: Keep in mind, libbpf can modify the program's instructions
    /// and consequently its instruction count, as it processes the BPF object file.
    /// So [`OpenProgram::insn_cnt`] and [`Program::insn_cnt`] may return different values.
    ///
    pub fn insn_cnt(&self) -> usize {
        unsafe { libbpf_sys::bpf_program__insn_cnt(self.ptr.as_ptr()) as usize }
    }

    /// Gives read-only access to BPF program's underlying BPF instructions.
    ///
    /// Keep in mind, libbpf can modify and append/delete BPF program's
    /// instructions as it processes BPF object file and prepares everything for
    /// uploading into the kernel. So [`OpenProgram::insns`] and [`Program::insns`] may return
    /// different sets of instructions. As an example, during BPF object load phase BPF program
    /// instructions will be CO-RE-relocated, BPF subprograms instructions will be appended, ldimm64
    /// instructions will have FDs embedded, etc. So instructions returned before load and after it
    /// might be quite different.
    pub fn insns(&self) -> &[libbpf_sys::bpf_insn] {
        let count = self.insn_cnt();
        let ptr = unsafe { libbpf_sys::bpf_program__insns(self.ptr.as_ptr()) };
        unsafe { slice::from_raw_parts(ptr, count) }
    }
}

impl AsRawLibbpf for Program {
    type LibbpfType = libbpf_sys::bpf_program;

    /// Retrieve the underlying [`libbpf_sys::bpf_program`].
    fn as_libbpf_object(&self) -> NonNull<Self::LibbpfType> {
        self.ptr
    }
}

/// Type of a [`Program`]. Maps to `enum bpf_prog_type` in kernel uapi.
#[non_exhaustive]
#[repr(u32)]
#[derive(Copy, Clone, TryFromPrimitive, Display, Debug)]
// TODO: Document variants.
#[allow(missing_docs)]
pub enum ProgramType {
    Unspec = 0,
    SocketFilter,
    Kprobe,
    SchedCls,
    SchedAct,
    Tracepoint,
    Xdp,
    PerfEvent,
    CgroupSkb,
    CgroupSock,
    LwtIn,
    LwtOut,
    LwtXmit,
    SockOps,
    SkSkb,
    CgroupDevice,
    SkMsg,
    RawTracepoint,
    CgroupSockAddr,
    LwtSeg6local,
    LircMode2,
    SkReuseport,
    FlowDissector,
    CgroupSysctl,
    RawTracepointWritable,
    CgroupSockopt,
    Tracing,
    StructOps,
    Ext,
    Lsm,
    SkLookup,
    Syscall,
    /// See [`MapType::Unknown`][crate::MapType::Unknown]
    Unknown = u32::MAX,
}

impl ProgramType {
    /// Detects if host kernel supports this BPF program type
    ///
    /// Make sure the process has required set of CAP_* permissions (or runs as
    /// root) when performing feature checking.
    pub fn is_supported(&self) -> Result<bool> {
        let ret = unsafe { libbpf_sys::libbpf_probe_bpf_prog_type(*self as u32, ptr::null()) };
        match ret {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::from_raw_os_error(-ret)),
        }
    }

    /// Detects if host kernel supports the use of a given BPF helper from this BPF program type.
    /// * `helper_id` - BPF helper ID (enum bpf_func_id) to check support for
    ///
    /// Make sure the process has required set of CAP_* permissions (or runs as
    /// root) when performing feature checking.
    pub fn is_helper_supported(&self, helper_id: bpf_func_id) -> Result<bool> {
        let ret =
            unsafe { libbpf_sys::libbpf_probe_bpf_helper(*self as u32, helper_id, ptr::null()) };
        match ret {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::from_raw_os_error(-ret)),
        }
    }
}

/// Attach type of a [`Program`]. Maps to `enum bpf_attach_type` in kernel uapi.
#[non_exhaustive]
#[repr(u32)]
#[derive(Clone, TryFromPrimitive, Display, Debug)]
// TODO: Document variants.
#[allow(missing_docs)]
pub enum ProgramAttachType {
    CgroupInetIngress,
    CgroupInetEgress,
    CgroupInetSockCreate,
    CgroupSockOps,
    SkSkbStreamParser,
    SkSkbStreamVerdict,
    CgroupDevice,
    SkMsgVerdict,
    CgroupInet4Bind,
    CgroupInet6Bind,
    CgroupInet4Connect,
    CgroupInet6Connect,
    CgroupInet4PostBind,
    CgroupInet6PostBind,
    CgroupUdp4Sendmsg,
    CgroupUdp6Sendmsg,
    LircMode2,
    FlowDissector,
    CgroupSysctl,
    CgroupUdp4Recvmsg,
    CgroupUdp6Recvmsg,
    CgroupGetsockopt,
    CgroupSetsockopt,
    TraceRawTp,
    TraceFentry,
    TraceFexit,
    ModifyReturn,
    LsmMac,
    TraceIter,
    CgroupInet4Getpeername,
    CgroupInet6Getpeername,
    CgroupInet4Getsockname,
    CgroupInet6Getsockname,
    XdpDevmap,
    CgroupInetSockRelease,
    XdpCpumap,
    SkLookup,
    Xdp,
    SkSkbVerdict,
    SkReuseportSelect,
    SkReuseportSelectOrMigrate,
    PerfEvent,
    /// See [`MapType::Unknown`][crate::MapType::Unknown]
    Unknown = u32::MAX,
}

/// The input a program accepts.
///
/// This type is mostly used in conjunction with the [`Program::test_run`]
/// facility.
#[derive(Debug, Default)]
pub struct Input<'dat> {
    /// The input context to provide.
    pub context_in: Option<&'dat [u8]>,
    /// The output context buffer provided to the program.
    pub context_out: Option<&'dat mut [u8]>,
    /// Additional data to provide to the program.
    pub data_in: Option<&'dat [u8]>,
    /// The output data buffer provided to the program.
    pub data_out: Option<&'dat mut [u8]>,
    /// The 'cpu' value passed to the kernel.
    pub cpu: u32,
    /// The 'flags' value passed to the kernel.
    pub flags: u32,
    /// The struct is non-exhaustive and open to extension.
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

/// The output a program produces.
///
/// This type is mostly used in conjunction with the [`Program::test_run`]
/// facility.
#[derive(Debug)]
pub struct Output<'dat> {
    /// The value returned by the program.
    pub return_value: u32,
    /// The output context filled by the program/kernel.
    pub context: Option<&'dat mut [u8]>,
    /// Output data filled by the program.
    pub data: Option<&'dat mut [u8]>,
    /// The struct is non-exhaustive and open to extension.
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

/// Represents a loaded [`Program`].
///
/// This struct is not safe to clone because the underlying libbpf resource cannot currently
/// be protected from data races.
///
/// If you attempt to attach a `Program` with the wrong attach method, the `attach_*`
/// method will fail with the appropriate error.
#[derive(Debug)]
pub struct Program {
    pub(crate) ptr: NonNull<libbpf_sys::bpf_program>,
    name: String,
    section: String,
}

impl AsFd for Program {
    fn as_fd(&self) -> BorrowedFd<'_> {
        let fd = unsafe { libbpf_sys::bpf_program__fd(self.ptr.as_ptr()) };
        unsafe { BorrowedFd::borrow_raw(fd) }
    }
}

impl Program {
    /// Create a [`Program`] from a [`libbpf_sys::bpf_program`]
    ///
    /// # Safety
    /// The pointer must point to a loaded program.
    pub(crate) unsafe fn new(ptr: NonNull<libbpf_sys::bpf_program>) -> Result<Self> {
        // Get the program name
        // bpf_program__name never returns NULL, so no need to check the pointer.
        let name = unsafe { libbpf_sys::bpf_program__name(ptr.as_ptr()) };
        let name = util::c_ptr_to_string(name)?;

        // Get the program section
        // bpf_program__section_name never returns NULL, so no need to check the pointer.
        let section = unsafe { libbpf_sys::bpf_program__section_name(ptr.as_ptr()) };
        let section = util::c_ptr_to_string(section)?;

        Ok(Program { ptr, name, section })
    }

    /// Retrieve the program's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Name of the section this `Program` belongs to.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// Retrieve the type of the program.
    pub fn prog_type(&self) -> ProgramType {
        match ProgramType::try_from(unsafe { libbpf_sys::bpf_program__type(self.ptr.as_ptr()) }) {
            Ok(ty) => ty,
            Err(_) => ProgramType::Unknown,
        }
    }

    /// Returns program fd by id
    pub fn get_fd_by_id(id: u32) -> Result<OwnedFd> {
        let ret = unsafe { libbpf_sys::bpf_prog_get_fd_by_id(id) };
        let fd = util::parse_ret_i32(ret)?;
        // SAFETY
        // A file descriptor coming from the bpf_prog_get_fd_by_id function is always suitable for
        // ownership and can be cleaned up with close.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// Returns program id by fd
    pub fn get_id_by_fd(fd: BorrowedFd<'_>) -> Result<u32> {
        let mut prog_info = libbpf_sys::bpf_prog_info::default();
        let prog_info_ptr: *mut libbpf_sys::bpf_prog_info = &mut prog_info;
        let mut len = size_of::<libbpf_sys::bpf_prog_info>() as u32;
        let ret = unsafe {
            libbpf_sys::bpf_obj_get_info_by_fd(
                fd.as_raw_fd(),
                prog_info_ptr as *mut c_void,
                &mut len,
            )
        };
        util::parse_ret(ret)?;
        Ok(prog_info.id)
    }

    /// Returns flags that have been set for the program.
    pub fn flags(&self) -> u32 {
        unsafe { libbpf_sys::bpf_program__flags(self.ptr.as_ptr()) }
    }

    /// Retrieve the attach type of the program.
    pub fn attach_type(&self) -> ProgramAttachType {
        match ProgramAttachType::try_from(unsafe {
            libbpf_sys::bpf_program__expected_attach_type(self.ptr.as_ptr())
        }) {
            Ok(ty) => ty,
            Err(_) => ProgramAttachType::Unknown,
        }
    }

    /// Return `true` if the bpf program is set to autoload, `false` otherwise.
    pub fn autoload(&self) -> bool {
        unsafe { libbpf_sys::bpf_program__autoload(self.ptr.as_ptr()) }
    }

    /// Return the bpf program's log level.
    pub fn log_level(&self) -> u32 {
        unsafe { libbpf_sys::bpf_program__log_level(self.ptr.as_ptr()) }
    }

    /// [Pin](https://facebookmicrosites.github.io/bpf/blog/2018/08/31/object-lifetime.html#bpffs)
    /// this program to bpffs.
    pub fn pin<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_c = util::path_to_cstring(path)?;
        let path_ptr = path_c.as_ptr();

        let ret = unsafe { libbpf_sys::bpf_program__pin(self.ptr.as_ptr(), path_ptr) };
        util::parse_ret(ret)
    }

    /// [Unpin](https://facebookmicrosites.github.io/bpf/blog/2018/08/31/object-lifetime.html#bpffs)
    /// this program from bpffs
    pub fn unpin<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path_c = util::path_to_cstring(path)?;
        let path_ptr = path_c.as_ptr();

        let ret = unsafe { libbpf_sys::bpf_program__unpin(self.ptr.as_ptr(), path_ptr) };
        util::parse_ret(ret)
    }

    /// Auto-attach based on prog section
    pub fn attach(&mut self) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach(self.ptr.as_ptr())
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a
    /// [cgroup](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html).
    pub fn attach_cgroup(&mut self, cgroup_fd: i32) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_cgroup(self.ptr.as_ptr(), cgroup_fd)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [perf event](https://linux.die.net/man/2/perf_event_open).
    pub fn attach_perf_event(&mut self, pfd: i32) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_perf_event(self.ptr.as_ptr(), pfd)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [userspace
    /// probe](https://www.kernel.org/doc/html/latest/trace/uprobetracer.html).
    pub fn attach_uprobe<T: AsRef<Path>>(
        &mut self,
        retprobe: bool,
        pid: i32,
        binary_path: T,
        func_offset: usize,
    ) -> Result<Link> {
        let path = util::path_to_cstring(binary_path)?;
        let path_ptr = path.as_ptr();
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_uprobe(
                self.ptr.as_ptr(),
                retprobe,
                pid,
                path_ptr,
                func_offset as libbpf_sys::size_t,
            )
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [userspace
    /// probe](https://www.kernel.org/doc/html/latest/trace/uprobetracer.html),
    /// providing additional options.
    pub fn attach_uprobe_with_opts(
        &mut self,
        pid: i32,
        binary_path: impl AsRef<Path>,
        func_offset: usize,
        opts: UprobeOpts,
    ) -> Result<Link> {
        let path = util::path_to_cstring(binary_path)?;
        let path_ptr = path.as_ptr();
        let UprobeOpts {
            ref_ctr_offset,
            cookie,
            retprobe,
            func_name,
            _non_exhaustive,
        } = opts;

        let func_name = util::str_to_cstring(&func_name)?;
        let opts = libbpf_sys::bpf_uprobe_opts {
            sz: size_of::<libbpf_sys::bpf_uprobe_opts>() as _,
            ref_ctr_offset: ref_ctr_offset as libbpf_sys::size_t,
            bpf_cookie: cookie,
            retprobe,
            func_name: func_name.as_ptr(),
            ..Default::default()
        };

        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_uprobe_opts(
                self.ptr.as_ptr(),
                pid,
                path_ptr,
                func_offset as libbpf_sys::size_t,
                &opts as *const _,
            )
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [kernel
    /// probe](https://www.kernel.org/doc/html/latest/trace/kprobetrace.html).
    pub fn attach_kprobe<T: AsRef<str>>(&mut self, retprobe: bool, func_name: T) -> Result<Link> {
        let func_name = util::str_to_cstring(func_name.as_ref())?;
        let func_name_ptr = func_name.as_ptr();
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_kprobe(self.ptr.as_ptr(), retprobe, func_name_ptr)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to the specified syscall
    pub fn attach_ksyscall<T: AsRef<str>>(
        &mut self,
        retprobe: bool,
        syscall_name: T,
    ) -> Result<Link> {
        let opts = libbpf_sys::bpf_ksyscall_opts {
            sz: size_of::<libbpf_sys::bpf_ksyscall_opts>() as _,
            retprobe,
            ..Default::default()
        };

        let syscall_name = util::str_to_cstring(syscall_name.as_ref())?;
        let syscall_name_ptr = syscall_name.as_ptr();
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_ksyscall(self.ptr.as_ptr(), syscall_name_ptr, &opts)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    fn attach_tracepoint_impl(
        &mut self,
        tp_category: &str,
        tp_name: &str,
        tp_opts: Option<TracepointOpts>,
    ) -> Result<Link> {
        let tp_category = util::str_to_cstring(tp_category)?;
        let tp_category_ptr = tp_category.as_ptr();
        let tp_name = util::str_to_cstring(tp_name)?;
        let tp_name_ptr = tp_name.as_ptr();

        util::create_bpf_entity_checked(|| {
            if let Some(tp_opts) = tp_opts {
                let tp_opts = libbpf_sys::bpf_tracepoint_opts::from(tp_opts);
                unsafe {
                    libbpf_sys::bpf_program__attach_tracepoint_opts(
                        self.ptr.as_ptr(),
                        tp_category_ptr,
                        tp_name_ptr,
                        &tp_opts as *const _,
                    )
                }
            } else {
                unsafe {
                    libbpf_sys::bpf_program__attach_tracepoint(
                        self.ptr.as_ptr(),
                        tp_category_ptr,
                        tp_name_ptr,
                    )
                }
            }
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [kernel
    /// tracepoint](https://www.kernel.org/doc/html/latest/trace/tracepoints.html).
    pub fn attach_tracepoint(
        &mut self,
        tp_category: impl AsRef<str>,
        tp_name: impl AsRef<str>,
    ) -> Result<Link> {
        self.attach_tracepoint_impl(tp_category.as_ref(), tp_name.as_ref(), None)
    }

    /// Attach this program to a [kernel
    /// tracepoint](https://www.kernel.org/doc/html/latest/trace/tracepoints.html),
    /// providing additional options.
    pub fn attach_tracepoint_with_opts(
        &mut self,
        tp_category: impl AsRef<str>,
        tp_name: impl AsRef<str>,
        tp_opts: TracepointOpts,
    ) -> Result<Link> {
        self.attach_tracepoint_impl(tp_category.as_ref(), tp_name.as_ref(), Some(tp_opts))
    }

    /// Attach this program to a [raw kernel
    /// tracepoint](https://lwn.net/Articles/748352/).
    pub fn attach_raw_tracepoint<T: AsRef<str>>(&mut self, tp_name: T) -> Result<Link> {
        let tp_name = util::str_to_cstring(tp_name.as_ref())?;
        let tp_name_ptr = tp_name.as_ptr();
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_raw_tracepoint(self.ptr.as_ptr(), tp_name_ptr)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach to an [LSM](https://en.wikipedia.org/wiki/Linux_Security_Modules) hook
    pub fn attach_lsm(&mut self) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_lsm(self.ptr.as_ptr())
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach to a [fentry/fexit kernel probe](https://lwn.net/Articles/801479/)
    pub fn attach_trace(&mut self) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_trace(self.ptr.as_ptr())
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach a verdict/parser to a [sockmap/sockhash](https://lwn.net/Articles/731133/)
    pub fn attach_sockmap(&self, map_fd: i32) -> Result<()> {
        let err = unsafe {
            libbpf_sys::bpf_prog_attach(
                self.as_fd().as_raw_fd(),
                map_fd,
                self.attach_type() as u32,
                0,
            )
        };
        util::parse_ret(err)
    }

    /// Attach this program to [XDP](https://lwn.net/Articles/825998/)
    pub fn attach_xdp(&mut self, ifindex: i32) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_xdp(self.ptr.as_ptr(), ifindex)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to [netns-based programs](https://lwn.net/Articles/819618/)
    pub fn attach_netns(&mut self, netns_fd: i32) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_netns(self.ptr.as_ptr(), netns_fd)
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    fn attach_usdt_impl(
        &mut self,
        pid: i32,
        binary_path: &Path,
        usdt_provider: &str,
        usdt_name: &str,
        usdt_opts: Option<UsdtOpts>,
    ) -> Result<Link> {
        let path = util::path_to_cstring(binary_path)?;
        let path_ptr = path.as_ptr();
        let usdt_provider = util::str_to_cstring(usdt_provider)?;
        let usdt_provider_ptr = usdt_provider.as_ptr();
        let usdt_name = util::str_to_cstring(usdt_name)?;
        let usdt_name_ptr = usdt_name.as_ptr();
        let usdt_opts = usdt_opts.map(libbpf_sys::bpf_usdt_opts::from);
        let usdt_opts_ptr = usdt_opts
            .as_ref()
            .map(|opts| opts as *const _)
            .unwrap_or_else(ptr::null);

        util::create_bpf_entity_checked(|| unsafe {
            libbpf_sys::bpf_program__attach_usdt(
                self.ptr.as_ptr(),
                pid,
                path_ptr,
                usdt_provider_ptr,
                usdt_name_ptr,
                usdt_opts_ptr,
            )
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Attach this program to a [USDT](https://lwn.net/Articles/753601/) probe
    /// point. The entry point of the program must be defined with
    /// `SEC("usdt")`.
    pub fn attach_usdt(
        &mut self,
        pid: i32,
        binary_path: impl AsRef<Path>,
        usdt_provider: impl AsRef<str>,
        usdt_name: impl AsRef<str>,
    ) -> Result<Link> {
        self.attach_usdt_impl(
            pid,
            binary_path.as_ref(),
            usdt_provider.as_ref(),
            usdt_name.as_ref(),
            None,
        )
    }

    /// Attach this program to a [USDT](https://lwn.net/Articles/753601/) probe
    /// point, providing additional options. The entry point of the program must
    /// be defined with `SEC("usdt")`.
    pub fn attach_usdt_with_opts(
        &mut self,
        pid: i32,
        binary_path: impl AsRef<Path>,
        usdt_provider: impl AsRef<str>,
        usdt_name: impl AsRef<str>,
        usdt_opts: UsdtOpts,
    ) -> Result<Link> {
        self.attach_usdt_impl(
            pid,
            binary_path.as_ref(),
            usdt_provider.as_ref(),
            usdt_name.as_ref(),
            Some(usdt_opts),
        )
    }

    /// Attach this program to a
    /// [BPF Iterator](https://www.kernel.org/doc/html/latest/bpf/bpf_iterators.html).
    /// The entry point of the program must be defined with `SEC("iter")` or `SEC("iter.s")`.
    pub fn attach_iter(&mut self, map_fd: BorrowedFd<'_>) -> Result<Link> {
        util::create_bpf_entity_checked(|| unsafe {
            let mut linkinfo = libbpf_sys::bpf_iter_link_info::default();
            linkinfo.map.map_fd = map_fd.as_raw_fd() as _;
            let attach_opt = libbpf_sys::bpf_iter_attach_opts {
                link_info: &mut linkinfo as *mut libbpf_sys::bpf_iter_link_info,
                link_info_len: size_of::<libbpf_sys::bpf_iter_link_info>() as _,
                sz: size_of::<libbpf_sys::bpf_iter_attach_opts>() as _,
                ..Default::default()
            };

            libbpf_sys::bpf_program__attach_iter(
                self.ptr.as_ptr(),
                &attach_opt as *const libbpf_sys::bpf_iter_attach_opts,
            )
        })
        .map(|ptr| unsafe {
            // SAFETY: the pointer came from libbpf and has been checked for errors
            Link::new(ptr)
        })
    }

    /// Test run the program with the given input data.
    ///
    /// This function uses the
    /// [BPF_PROG_RUN](https://www.kernel.org/doc/html/latest/bpf/bpf_prog_run.html)
    /// facility.
    pub fn test_run<'dat>(&mut self, input: Input<'dat>) -> Result<Output<'dat>> {
        pub(crate) unsafe fn slice_from_array<'t, T>(
            items: *mut T,
            num_items: usize,
        ) -> Option<&'t mut [T]> {
            if items.is_null() {
                None
            } else {
                Some(unsafe { slice::from_raw_parts_mut(items, num_items) })
            }
        }

        let Input {
            context_in,
            mut context_out,
            data_in,
            mut data_out,
            cpu,
            flags,
            _non_exhaustive: (),
        } = input;

        let mut opts = unsafe { mem::zeroed::<libbpf_sys::bpf_test_run_opts>() };
        opts.sz = size_of_val(&opts) as _;
        opts.ctx_in = context_in
            .map(|data| data.as_ptr().cast())
            .unwrap_or_else(ptr::null);
        opts.ctx_size_in = context_in.map(|data| data.len() as _).unwrap_or(0);
        opts.ctx_out = context_out
            .as_mut()
            .map(|data| data.as_mut_ptr().cast())
            .unwrap_or_else(ptr::null_mut);
        opts.ctx_size_out = context_out.map(|data| data.len() as _).unwrap_or(0);
        opts.data_in = data_in
            .map(|data| data.as_ptr().cast())
            .unwrap_or_else(ptr::null);
        opts.data_size_in = data_in.map(|data| data.len() as _).unwrap_or(0);
        opts.data_out = data_out
            .as_mut()
            .map(|data| data.as_mut_ptr().cast())
            .unwrap_or_else(ptr::null_mut);
        opts.data_size_out = data_out.map(|data| data.len() as _).unwrap_or(0);
        opts.cpu = cpu;
        opts.flags = flags;

        let rc = unsafe { libbpf_sys::bpf_prog_test_run_opts(self.as_fd().as_raw_fd(), &mut opts) };
        let () = util::parse_ret(rc)?;
        let output = Output {
            return_value: opts.retval,
            context: unsafe { slice_from_array(opts.ctx_out.cast(), opts.ctx_size_out as _) },
            data: unsafe { slice_from_array(opts.data_out.cast(), opts.data_size_out as _) },
            _non_exhaustive: (),
        };
        Ok(output)
    }

    /// Returns the number of instructions that form the program.
    ///
    /// Please see note in [`OpenProgram::insn_cnt`].
    pub fn insn_cnt(&self) -> usize {
        unsafe { libbpf_sys::bpf_program__insn_cnt(self.ptr.as_ptr()) as usize }
    }

    /// Gives read-only access to BPF program's underlying BPF instructions.
    ///
    /// Please see note in [`OpenProgram::insns`].
    ///
    pub fn insns(&self) -> &[libbpf_sys::bpf_insn] {
        let count = self.insn_cnt();
        let ptr = unsafe { libbpf_sys::bpf_program__insns(self.ptr.as_ptr()) };
        unsafe { slice::from_raw_parts(ptr, count) }
    }
}
