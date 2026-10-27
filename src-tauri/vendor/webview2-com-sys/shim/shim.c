// MSVC CRT 符号 shim：让 MinGW/GNU 工具链能静态链接 WebView2LoaderStatic.lib
//
// WebView2LoaderStatic.lib 由 MSVC 编译，依赖以下 MSVC 专有符号，GNU 工具链不提供：
//   1. /GS 栈保护：__security_cookie / __security_check_cookie
//   2. magic statics 线程安全初始化：_Init_thread_header / _Init_thread_footer / _Init_thread_epoch(TLS)
//   3. C++ 运算符（MSVC 修饰名）：operator new/delete/delete[]/new[](nothrow) 与 std::nothrow 对象
// 本文件为全部缺失符号提供等价实现，按架构条件编译，供三架构（x64/x86/arm64）使用。

#define _WIN32_WINNT 0x0601
#include <windows.h>
#include <stdlib.h>

/* ---------- 1. /GS 栈保护 ---------- */
/* cookie 仅需非零即可；校验函数做成空操作，等效于关闭栈保护，不影响功能正确性 */
#if defined(_WIN64) /* x64 与 arm64：cookie 8 字节，符号不修饰 */
unsigned long long __security_cookie = 0x0002B992DDFA2329ULL;
void __security_check_cookie(unsigned long long cookie) { (void)cookie; }
#else /* x86：cookie 4 字节，校验函数为 __fastcall（参数经 ECX），符号 @name@4 */
unsigned long __security_cookie = 0xBB40E64EUL;
void __fastcall __security_check_cookie(unsigned long cookie) { (void)cookie; }

/* MSVC x86 的 TLS 访问模式：mov edx, fs:[__tls_array]
 * __tls_array 即 TEB 中 ThreadLocalStoragePointer 字段的偏移（0x2C），
 * 链接器需提供该符号，其地址作为 fs 段内位移使用 */
const unsigned long __tls_array = 0x2C;
#endif

/* ---------- 2. magic statics（vcruntime 线程安全静态初始化） ---------- */
/* _Init_thread_epoch 必须是真正的 PE TLS 变量：
 * MSVC 生成的代码通过 TLS 数组 + __tls_index + SECREL 偏移访问它，
 * 普通全局变量无法满足重定位语义，故放入 .tls 段（mingw ld 会并入 PE TLS 目录） */
__attribute__((section(".tls"))) volatile int _Init_thread_epoch = 0;

/* 全局初始化纪元：MSVC 库本体并不引用此符号（其 magic statics 仅依赖 TLS epoch），
 * 由本 shim 自行定义，供 header/footer 维护 */
int _Init_global_epoch = 0;

static SRWLOCK g_init_lock = SRWLOCK_INIT;
static CONDITION_VARIABLE g_init_cv = CONDITION_VARIABLE_INIT;

/* 返回 1：调用方执行静态初始化；返回 0：其他线程已完成 */
int _Init_thread_header(volatile int *pOnce)
{
    AcquireSRWLockExclusive(&g_init_lock);
    if (*pOnce == -1)
    {
        /* 已完成，刷新本线程 epoch，之后走快速路径 */
        _Init_thread_epoch = _Init_global_epoch;
        ReleaseSRWLockExclusive(&g_init_lock);
        return 0;
    }
    if (*pOnce == 0)
    {
        /* 本线程抢到初始化权 */
        *pOnce = 1;
        ReleaseSRWLockExclusive(&g_init_lock);
        return 1;
    }
    /* 其他线程正在初始化，等待其完成 */
    while (*pOnce != -1)
    {
        SleepConditionVariableSRW(&g_init_cv, &g_init_lock, INFINITE, 0);
    }
    _Init_thread_epoch = _Init_global_epoch;
    ReleaseSRWLockExclusive(&g_init_lock);
    return 0;
}

/* 标记初始化完成：guard 置 -1 并推进全局 epoch，唤醒等待线程 */
void _Init_thread_footer(volatile int *pOnce)
{
    AcquireSRWLockExclusive(&g_init_lock);
    *pOnce = -1;
    _Init_global_epoch++;
    WakeAllConditionVariable(&g_init_cv);
    ReleaseSRWLockExclusive(&g_init_lock);
    _Init_thread_epoch = _Init_global_epoch;
}

/* ---------- 3. C++ 运算符 ----------
 * GNU as 不接受 '?' 开头的符号名，无法用 asm 标签直接命名，
 * 这里使用普通名称编译，构建脚本用 objcopy --redefine-sym 重命名为 MSVC 修饰名 */
#if defined(_WIN64)
/* x64/arm64：size_t 为 8 字节 */
void msvc_op_delete(void *p) { free(p); }

void msvc_op_delete_array(void *p) { free(p); }

/* OOM 时直接终止（等效于抛出 bad_alloc 后的终止语义） */
void *msvc_op_new_array(unsigned long long n)
{
    void *p = malloc(n ? (size_t)n : 1);
    if (!p)
        abort();
    return p;
}

/* 失败返回 nullptr */
void *msvc_op_new_nothrow(unsigned long long n, const void *tag)
{
    (void)tag;
    return malloc(n ? (size_t)n : 1);
}

/* const std::nothrow_t std::nothrow 对象本体（空结构，1 字节占位） */
const char std_nothrow_obj = 0;

#else
/* x86：size_t 为 4 字节 */
void msvc_op_delete(void *p) { free(p); }

void msvc_op_delete_array(void *p) { free(p); }

void *msvc_op_new_array(unsigned int n)
{
    void *p = malloc(n ? (size_t)n : 1);
    if (!p)
        abort();
    return p;
}

void *msvc_op_new_nothrow(unsigned int n, const void *tag)
{
    (void)tag;
    return malloc(n ? (size_t)n : 1);
}

const char std_nothrow_obj = 0;
#endif
