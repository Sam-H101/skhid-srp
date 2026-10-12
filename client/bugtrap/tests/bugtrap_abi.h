// SPDX-License-Identifier: GPL-3.0-or-later
// Minimal native ANSI BugTrap declarations used by the independent C ABI probe.
#pragma once
#include <windows.h>
#include <dbghelp.h>
#define IDC_INTRO2 1031
#define IDC_SUBMIT_BUG 1013
#define MiniDumpNoDump MAXDWORD
#define BTF_NONE 0
#define BTA_SHOWUI 1
#define BTA_SAVEREPORT 2
#define BTEM_EXECUTEHANDLER 2
#define BTEM_CONTINUESEARCH 1
#ifdef __cplusplus
extern "C" {
#endif
__declspec(dllimport) void WINAPI BT_SetAppName(LPCSTR);
__declspec(dllimport) LPCSTR WINAPI BT_GetAppName(void);
__declspec(dllimport) void WINAPI BT_SetUserMessage(LPCSTR);
__declspec(dllimport) void WINAPI BT_SetSupportEMail(LPCSTR);
__declspec(dllimport) void WINAPI BT_SetSupportServer(LPCSTR, SHORT);
__declspec(dllimport) void WINAPI BT_SetSupportURL(LPCSTR);
__declspec(dllimport) LPCSTR WINAPI BT_GetSupportEMail(void);
__declspec(dllimport) LPCSTR WINAPI BT_GetSupportHost(void);
__declspec(dllimport) LPCSTR WINAPI BT_GetSupportURL(void);
__declspec(dllimport) void WINAPI BT_SetDumpType(DWORD);
__declspec(dllimport) void WINAPI BT_SetFlags(DWORD);
__declspec(dllimport) void WINAPI BT_SetActivityType(DWORD);
__declspec(dllimport) void WINAPI BT_SetExitMode(DWORD);
__declspec(dllimport) LPTOP_LEVEL_EXCEPTION_FILTER WINAPI BT_InstallSehFilter(void);
__declspec(dllimport) void WINAPI BT_SetPreErrHandler(void (CALLBACK *)(INT_PTR), INT_PTR);
__declspec(dllimport) LONG WINAPI BT_SehFilter(PEXCEPTION_POINTERS);
__declspec(dllimport) void __cdecl BT_CallCppFilter(void);
__declspec(dllimport) void __cdecl BT_CallSehFilter(void);
__declspec(dllimport) void __cdecl BT_CallNetFilter(void);
__declspec(dllimport) BOOL WINAPI BT_SaveSnapshot(LPCSTR);
__declspec(dllimport) BOOL WINAPI BT_SendSnapshot(void);
__declspec(dllimport) BOOL WINAPI BT_SendSnapshotEx(PEXCEPTION_POINTERS);
__declspec(dllimport) BOOL WINAPI BT_MailSnapshot(void);
__declspec(dllimport) BOOL WINAPI BT_MailSnapshotEx(PEXCEPTION_POINTERS);
#ifdef __cplusplus
}
#endif
