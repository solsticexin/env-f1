/* memory.x */
MEMORY {
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 64K
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 20K
}

/* 显式分配堆栈和堆 */
_stack_size = 4K;  /* 增大堆栈到 4KB */
_heap_size = 1K;   /* 可选：分配 1KB 堆 */

_stack_start = ORIGIN(RAM) + LENGTH(RAM);
_stack_end = _stack_start - _stack_size;
_heap_start = _stack_end - _heap_size;
_heap_end = _stack_end;
