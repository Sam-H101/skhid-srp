#include <windows.h>

static DWORD WINAPI CrashWorker(void* parameter)
{
    HANDLE event = (HANDLE)parameter;
    char path[MAX_PATH];
    DWORD length = GetEnvironmentVariableA("SRP_CRASH_TEST_THREAD_FILE", path, sizeof(path));
    if (!length || length >= sizeof(path)) return 1;
    HANDLE file = CreateFileA(path, GENERIC_WRITE, FILE_SHARE_READ, NULL, CREATE_ALWAYS, 0, NULL);
    if (file != INVALID_HANDLE_VALUE) {
        char text[64]; DWORD written;
        int size = wsprintfA(text, "%lu\n", GetCurrentThreadId());
        WriteFile(file, text, size, &written, NULL);
        CloseHandle(file);
    }
    WaitForSingleObject(event, INFINITE);
    CloseHandle(event);
    *(volatile DWORD*)0 = 0x26;
    return 0;
}

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, void*)
{
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(instance);
        char name[80];
        wsprintfA(name, "Local\\SRP_026_CrashTest_%lu", GetCurrentProcessId());
        HANDLE event = CreateEventA(NULL, TRUE, FALSE, name);
        if (!event) return FALSE;
        HANDLE thread = CreateThread(NULL, 256*1024, CrashWorker, event, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
        if (!thread) { CloseHandle(event); return FALSE; }
        CloseHandle(thread);
    }
    return TRUE;
}
