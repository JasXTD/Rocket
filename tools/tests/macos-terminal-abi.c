/* Compile/run with the installed Apple SDK before the native Rust PTY tests.
 * Independent SDK checks for Rocket's handwritten Darwin FFI; no downloaded C. */
#define _DARWIN_C_SOURCE 1
#include <stddef.h>
#include <stdint.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <sys/ttycom.h>
#include <sys/wait.h>
#include <sys/proc_info.h>
#include <termios.h>

_Static_assert(sizeof(struct termios) == 72, "Darwin termios layout");
_Static_assert(offsetof(struct termios, c_cc) == 32, "termios control characters");
_Static_assert(offsetof(struct termios, c_ispeed) == 56, "termios speed padding");
_Static_assert(sizeof(nfds_t) == 4, "poll count ABI");
_Static_assert(sizeof(siginfo_t) == 104 && offsetof(siginfo_t, si_pid) == 12, "waitid layout");
_Static_assert(sizeof(struct proc_bsdshortinfo) == 64, "libproc short info");
_Static_assert(PROC_PIDT_SHORTBSDINFO == 13 && PROC_UID_ONLY == 4 && PROC_PGRP_ONLY == 2, "libproc selectors");
_Static_assert(offsetof(struct proc_bsdshortinfo, pbsi_pgid) == 8 && offsetof(struct proc_bsdshortinfo, pbsi_status) == 12, "process group/status offsets");
_Static_assert(O_NOCTTY == 0x20000 && O_CLOEXEC == 0x1000000 && O_NONBLOCK == 4 && O_NOFOLLOW == 0x100, "open flags");
_Static_assert(TIOCSCTTY == 0x20007461 && TIOCSWINSZ == 0x80087467 && TIOCPTYGNAME == 0x40807453, "PTY ioctls");
_Static_assert(IUTF8 == 0x4000 && TCSANOW == 0, "terminal attributes");
_Static_assert(SIGCONT == 19 && SIGSTOP == 17 && P_PID == 1, "signal identifiers");
_Static_assert(WEXITED == 4 && WNOHANG == 1 && WNOWAIT == 0x20, "wait options");
_Static_assert(CLD_EXITED == 1 && CLD_KILLED == 2 && CLD_DUMPED == 3, "exit statuses");
int main(void) { return 0; }
