# Runs the battery rule's container event handler (0x313A50..0x313B3D of the
# 0.26g exe, patch 9 of README.md) as it stands in a given exe: the 238
# bytes are copied out of the file and called in this process on a made-up
# rule object. Nothing of the game is started and no file is written.
#
# It plays what match.md section 26 describes: nine batteries put in, the
# tenth, a rollback (which loads the counts and neither the completion time
# nor the "containers can be used" byte), the same put again, a take, a put.
# An exe without the patch does not count the replayed put and leaves the
# byte cleared; an exe with it counts every event and leaves the byte set.
#
# The code is 32-bit, so this needs the 32-bit Windows PowerShell:
#   C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe -NoProfile -ExecutionPolicy Bypass -File run_battery_rule.ps1 <exe> [<exe> ...]
# Any 0.26g exe will do: the original, a --cert-only build, a full build.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Exes)

if ([IntPtr]::Size -ne 4) { throw "this must run in a 32-bit PowerShell" }

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public static class Gather
{
    [DllImport("kernel32")] static extern IntPtr VirtualAlloc(IntPtr address, UIntPtr size, uint type, uint protect);

    [UnmanagedFunctionPointer(CallingConvention.ThisCall)]
    delegate void Handler(IntPtr rule, int kind, IntPtr data, int time);

    const int FileOffset = 0x302E50;   // VA 0x313A50
    const int Length = 0xEE;           // to the last `ret 0Ch`, 0x313B3B..0x313B3D

    static IntPtr rule, data, world;
    static Handler handler;
    static StringBuilder text;

    static void State(string what)
    {
        text.AppendFormat("  {0,-58} count {1,2}:{2}  completion {3,7}  containers usable {4}\n", what,
            Marshal.ReadByte(rule, 0x144), Marshal.ReadByte(rule, 0x145),
            Marshal.ReadInt32(rule, 0x15C), Marshal.ReadByte(rule, 0x160));
    }

    static void Event(int team, bool taken, int time)
    {
        Marshal.WriteInt32(data, 0, team);
        Marshal.WriteInt32(data, 4, taken ? 1 : 0);
        handler(rule, 2, data, time);
    }

    public static string Run(string exe)
    {
        byte[] file = File.ReadAllBytes(exe);
        byte[] code = new byte[Length];
        Array.Copy(file, FileOffset, code, 0, Length);
        IntPtr memory = VirtualAlloc(IntPtr.Zero, (UIntPtr)0x1000, 0x3000, 0x40);
        Marshal.Copy(code, 0, memory, Length);
        handler = (Handler)Marshal.GetDelegateForFunctionPointer(memory, typeof(Handler));

        // The rule: ten batteries (a vector of ten pointers at +0x110/+0x114),
        // the two counts at +0x144/+0x145, the world at +0x158 (its lobby
        // menu pointer at +0x18FA8 is left null: no HUD to update), the
        // completion time at +0x15C (-1: none), the "containers can be
        // used" byte at +0x160 (1, as the constructor sets it).
        rule = Marshal.AllocHGlobal(0x200);
        data = Marshal.AllocHGlobal(8);
        world = Marshal.AllocHGlobal(0x19000);
        for (int i = 0; i < 0x200; i += 4) Marshal.WriteInt32(rule, i, 0);
        for (int i = 0; i < 0x19000; i += 4) Marshal.WriteInt32(world, i, 0);
        IntPtr batteries = Marshal.AllocHGlobal(40);
        Marshal.WriteInt32(rule, 0x110, batteries.ToInt32());
        Marshal.WriteInt32(rule, 0x114, batteries.ToInt32() + 40);
        Marshal.WriteInt32(rule, 0x158, world.ToInt32());
        Marshal.WriteInt32(rule, 0x15C, -1);
        Marshal.WriteByte(rule, 0x160, 1);

        text = new StringBuilder();
        text.AppendFormat("{0}\n  handler bytes at 0x313A62: {1}  at 0x313B34: {2}\n", exe,
            BitConverter.ToString(code, 0x12, 6), BitConverter.ToString(code, 0xE4, 7));
        for (int i = 0; i < 9; i++) Event(0, false, 1000 * (i + 1));
        State("nine batteries put in by team 0");
        Event(0, false, 394700);
        State("the tenth put in at 394700 ms");
        // A rollback loads the counts (they are serialized) and neither the
        // completion time nor the byte.
        Marshal.WriteByte(rule, 0x144, 9);
        State("rollback to before that tick (count loaded: 9)");
        Event(0, false, 394700);
        State("the same tick played again: the put");
        Event(0, true, 395700);
        State("a battery taken out of the full container");
        Event(0, false, 396700);
        State("put in again");
        Marshal.WriteByte(rule, 0x144, 9);
        State("rollback with no replay of the put (the carrier died)");
        Event(1, false, 400000);
        State("the next event: team 1 puts one in");
        return text.ToString();
    }
}
'@

foreach ($exe in $Exes) { [Gather]::Run((Resolve-Path $exe).Path) }
