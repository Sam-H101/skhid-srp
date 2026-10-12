// Test-only forwarding shim. Hold the real writer behind a gate so a second
// crash reliably enters BugTrap while the first dump file is still open.
#include <windows.h>
#include <dbghelp.h>
#include <stdio.h>

extern "C" BOOL WINAPI ProbeMiniDumpWriteDump(HANDLE process, DWORD pid, HANDLE file,
    MINIDUMP_TYPE type, PMINIDUMP_EXCEPTION_INFORMATION exception,
    PMINIDUMP_USER_STREAM_INFORMATION streams, PMINIDUMP_CALLBACK_INFORMATION callback)
{
    char name[100];
    sprintf_s(name, "Local\\SRPProbeWriterStarted-%lu", GetCurrentProcessId());
    HANDLE started = OpenEventA(EVENT_MODIFY_STATE, FALSE, name);
    sprintf_s(name, "Local\\SRPProbeWriterRelease-%lu", GetCurrentProcessId());
    HANDLE release = OpenEventA(SYNCHRONIZE, FALSE, name);
    if (!started || !release) return FALSE;
    SetEvent(started);
    DWORD wait = WaitForSingleObject(release, 10000);
    CloseHandle(started);
    CloseHandle(release);
    if (wait != WAIT_OBJECT_0) return FALSE;

    WCHAR path[MAX_PATH];
    if (!GetSystemDirectoryW(path, MAX_PATH)) return FALSE;
    wcscat_s(path, L"\\dbghelp.dll");
    HMODULE real = LoadLibraryW(path);
    if (!real) return FALSE;
    typedef BOOL (WINAPI *Writer)(HANDLE, DWORD, HANDLE, MINIDUMP_TYPE,
        PMINIDUMP_EXCEPTION_INFORMATION, PMINIDUMP_USER_STREAM_INFORMATION,
        PMINIDUMP_CALLBACK_INFORMATION);
    Writer writer = (Writer)GetProcAddress(real, "MiniDumpWriteDump");
    BOOL result = writer && (void*)writer != (void*)ProbeMiniDumpWriteDump
        && writer(process, pid, file, type, exception, streams, callback);
    DWORD error = GetLastError();
    FreeLibrary(real);
    SetLastError(error);
    return result;
}
