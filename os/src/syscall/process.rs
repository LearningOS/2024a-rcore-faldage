//! Process management syscalls
use alloc::sync::Arc;

use core::{mem::size_of, ptr::copy_nonoverlapping};

use crate::{
    config::PAGE_SIZE_BITS,
    loader::get_app_data_by_name,
    mm::{translated_byte_buffer, translated_refmut, translated_str, MapPermission, VirtAddr},
    task::{
        add_task, check_vpn_allocated, current_task, current_user_token, exit_current_and_run_next,
        get_current_task_info, insert_framed_area_to_current_task,
        remove_framed_area_from_current_task, suspend_current_and_run_next, TaskControlBlock,
        TaskInfo,
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

fn copy_struct<T>(addr: *mut T, src: T) {
    let translated_addr =
        translated_byte_buffer(current_user_token(), addr as *const u8, size_of::<T>());
    if translated_addr.len() == 1 {
        unsafe {
            let paddr = translated_addr.get(0).unwrap().as_ptr() as *mut T;
            core::ptr::write(paddr, src);
        }
    } else {
        let mut data_pointer: *const u8 = &src as *const T as *const u8;
        for s in translated_addr {
            unsafe {
                copy_nonoverlapping(data_pointer, s.as_mut_ptr(), s.len());
            }
            data_pointer = data_pointer.wrapping_add(s.len());
        }
        panic!("struct is splitted by two pages")
    }
}
/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    copy_struct(
        _ts,
        TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        },
    );
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info NOT IMPLEMENTED YET!");
    copy_struct(_ti, get_current_task_info());
    0
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    let num = (1 << PAGE_SIZE_BITS) - 1;
    if start & num != 0 {
        return -1;
    }
    let start_va: VirtAddr = VirtAddr(start);
    let end_va: VirtAddr = VirtAddr(start + len);

    if check_vpn_allocated(start_va, end_va) {
        return -1;
    }

    //port & !0x7 != 0 (port 其余位必须为0)
    //port & 0x7 = 0 (这样的内存无意义)
    if port & !0x7 != 0 || port & 0x7 == 0 {
        return -1;
    }
    let mut permission = MapPermission::from_bits_truncate(0);
    if port & 1 != 0 {
        permission |= MapPermission::R;
    }
    if port & (1 << 1) != 0 {
        permission |= MapPermission::W;
    }
    if port & (1 << 2) != 0 {
        permission |= MapPermission::X;
    }
    permission |= MapPermission::U;

    insert_framed_area_to_current_task(start_va, end_va, permission);
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    let start_va: VirtAddr = VirtAddr(start);
    let end_va: VirtAddr = VirtAddr(start + len);
    remove_framed_area_from_current_task(start_va, end_va)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let cur_task = current_task().unwrap();
    let mut parent_inner = cur_task.inner_exclusive_access();

    let token = parent_inner.get_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let new_task_control_block = Arc::new(TaskControlBlock::new(data));
        parent_inner.children.push(new_task_control_block.clone());
        // modify kernel_sp in trap_cx
        // **** access child PCB exclusively
        let trap_cx = new_task_control_block
            .inner_exclusive_access()
            .get_trap_cx();
        trap_cx.kernel_sp = new_task_control_block.kernel_stack.get_top();
        trap_cx.x[10] = 0;
        add_task(new_task_control_block.clone());
        new_task_control_block.exec(data);
        new_task_control_block.getpid() as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let cur_task = current_task().unwrap();
    let mut parent_inner = cur_task.inner_exclusive_access();
    if prio >= 2 {
        parent_inner.set_priority(prio);
        prio
    } else {
        -1
    }
}
