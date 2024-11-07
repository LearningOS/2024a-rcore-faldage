//! Process management syscalls
use core::{mem::size_of, ptr::copy_nonoverlapping};

use crate::{
    config::PAGE_SIZE_BITS,
    mm::{translated_byte_buffer, MapPermission, VirtAddr},
    task::{
        change_program_brk, check_vpn_allocated, current_user_token, exit_current_and_run_next,
        get_current_task_info, insert_framed_area_to_current_task,
        remove_framed_area_from_current_task, suspend_current_and_run_next, TaskInfo,
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
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
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
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
