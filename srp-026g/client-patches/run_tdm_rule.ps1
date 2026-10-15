# Runs Team Deathmatch's death handler (tdm_rule_core, 0x316090, with the
# recount 0x3161F0 and the end test 0x316470 it calls) as it stands in a given
# exe (patch revision 11, README.md section 21): the code of 0x316000..0x317000
# is copied out of the file to the same addresses in this process and called
# on a made-up rule object, a world of four players in two teams and the
# statistics object. The flag setter 0x316630 (a blackboard write that needs
# the game's globals) is replaced by `ret 8`; nothing of the game is started
# and no file is written.
#
# It plays what match.md section 28 describes: a team spends its tickets, its
# players die one by one and leave the alive mask, the last one's death sets
# the rule's completion time (+0x12C) and ends the match; a rollback loads the
# tickets and the masks (serialized) and not the completion time; the replay
# of the same death; a death of the other team after that. An exe without the
# patch skips the replayed death (the bit stays, the tickets freeze); an exe
# with it counts every death and the bit is cleared again.
#
# The code is 32-bit, so this needs the 32-bit Windows PowerShell:
#   C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe -NoProfile -ExecutionPolicy Bypass -File run_tdm_rule.ps1 <exe> [<exe> ...]
# Any 0.26g exe will do: the original, a --cert-only build, a full build.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Exes)

if ([IntPtr]::Size -ne 4) { throw "this must run in a 32-bit PowerShell" }

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public static class Tdm
{
    [DllImport("kernel32")] static extern IntPtr VirtualAlloc(IntPtr address, UIntPtr size, uint type, uint protect);

    [UnmanagedFunctionPointer(CallingConvention.ThisCall)]
    delegate void Handler(IntPtr rule, int kind, IntPtr data, int time);

    const int TextVa = 0x11000, TextFile = 0x400;
    const int From = 0x316000, Length = 0x1000;
    const int HandlerVa = 0x316090, FlagSetterVa = 0x316630;

    static bool mapped;
    static IntPtr rule, data, world, stats;
    static Handler handler;
    static StringBuilder text;

    static string Bits(int mask)
    {
        string s = "";
        for (int i = 3; i >= 0; i--) s += ((mask >> i) & 1) == 1 ? "1" : "0";
        return s;
    }

    static void State(string what)
    {
        text.AppendFormat("  {0,-60} tickets {1,2}:{2,2}  alive {3} (counts {4}:{5})  completion {6,7}\n", what,
            Marshal.ReadInt16(rule, 0x110), Marshal.ReadInt16(rule, 0x112),
            Bits(Marshal.ReadInt32(rule, 0x120)), Marshal.ReadByte(rule, 0x11C), Marshal.ReadByte(rule, 0x11D),
            Marshal.ReadInt32(rule, 0x12C));
    }

    // A death of `victim` killed by `killer` (0xFF: nobody) at `time`.
    static void Death(int killer, int victim, int time)
    {
        Marshal.WriteByte(data, 0, (byte)killer);
        Marshal.WriteByte(data, 1, (byte)victim);
        handler(rule, 1, data, time);
    }

    // What a rollback to before a death loads: the tickets and both masks
    // (serialized). The completion time is not.
    static void Rollback(int tickets0, int tickets1, int alive)
    {
        Marshal.WriteInt16(rule, 0x110, (short)tickets0);
        Marshal.WriteInt16(rule, 0x112, (short)tickets1);
        Marshal.WriteInt32(rule, 0x120, alive);
    }

    public static string Run(string exe)
    {
        byte[] file = File.ReadAllBytes(exe);
        // The same addresses: the code calls and jumps relative and holds
        // absolute addresses of the image.
        if (!mapped)
        {
            IntPtr memory = VirtualAlloc((IntPtr)0x310000, (UIntPtr)0x10000, 0x3000, 0x40);
            if (memory.ToInt32() != 0x310000) throw new Exception("0x310000 is not free in this process");
            // Only one exe a process (see the end of the script).
            mapped = true;
        }
        Marshal.Copy(file, From - TextVa + TextFile, (IntPtr)From, Length);
        // The blackboard write of the respawn flag needs the game's globals.
        Marshal.WriteByte((IntPtr)FlagSetterVa, 0xC2);
        Marshal.WriteByte((IntPtr)(FlagSetterVa + 1), 0x08);
        Marshal.WriteByte((IntPtr)(FlagSetterVa + 2), 0x00);
        handler = (Handler)Marshal.GetDelegateForFunctionPointer((IntPtr)HandlerVa, typeof(Handler));

        rule = Marshal.AllocHGlobal(0x200);
        data = Marshal.AllocHGlobal(8);
        world = Marshal.AllocHGlobal(0x19000);
        stats = Marshal.AllocHGlobal(0x3200);
        for (int i = 0; i < 0x200; i += 4) Marshal.WriteInt32(rule, i, 0);
        for (int i = 0; i < 0x19000; i += 4) Marshal.WriteInt32(world, i, 0);
        for (int i = 0; i < 0x3200; i += 4) Marshal.WriteInt32(stats, i, 0);
        Marshal.WriteInt32(world, 0x18FA8, stats.ToInt32());

        // 20 entries of 0x30 bytes: byte +0xC the current kit, the kit's
        // player object at +kit*4. Four players: 0 and 1 in team 0, 2 and 3
        // in team 1. A player: the team's record at +0x6B8 (the team number
        // at +0x45EC of it), the place at +0x114, a part at +0x108 whose
        // vtable slot 2 gives the cell of a pointer to a buffer (the end
        // test writes the completion time into it).
        IntPtr table = Marshal.AllocHGlobal(20 * 0x30);
        for (int i = 0; i < 20 * 0x30; i += 4) Marshal.WriteInt32(table, i, 0);
        Marshal.WriteInt32(world, 0x18458, table.ToInt32());
        IntPtr buffer = Marshal.AllocHGlobal(0x400);
        for (int i = 0; i < 0x400; i += 4) Marshal.WriteInt32(buffer, i, 0);
        IntPtr cell = Marshal.AllocHGlobal(4);
        Marshal.WriteInt32(cell, 0, buffer.ToInt32());
        IntPtr getter = (IntPtr)(0x310000 + 0x8000);
        Marshal.WriteByte(getter, 0, 0xB8);
        Marshal.WriteInt32(getter, 1, cell.ToInt32());
        Marshal.WriteByte(getter, 5, 0xC3);
        IntPtr vtable = Marshal.AllocHGlobal(16);
        for (int i = 0; i < 16; i += 4) Marshal.WriteInt32(vtable, i, getter.ToInt32());
        for (int place = 0; place < 4; place++)
        {
            IntPtr team = Marshal.AllocHGlobal(0x4600);
            for (int i = 0; i < 0x4600; i += 4) Marshal.WriteInt32(team, i, 0);
            Marshal.WriteInt32(team, 0x45EC, place < 2 ? 0 : 1);
            IntPtr player = Marshal.AllocHGlobal(0x6C0);
            for (int i = 0; i < 0x6C0; i += 4) Marshal.WriteInt32(player, i, 0);
            Marshal.WriteInt32(player, 0x6B8, team.ToInt32());
            Marshal.WriteByte(player, 0x114, (byte)place);
            Marshal.WriteInt32(player, 0x108, vtable.ToInt32());
            Marshal.WriteInt32(table, place * 0x30, player.ToInt32());
        }

        // The rule: tickets at +0x110/+0x112, the blackboard at +0x118, the
        // masks at +0x120/+0x124, the world at +0x128, the completion time
        // at +0x12C (-1: none, as the constructor sets it).
        Marshal.WriteInt16(rule, 0x110, 5);
        Marshal.WriteInt16(rule, 0x112, 1);
        Marshal.WriteInt32(rule, 0x120, 0xF);
        Marshal.WriteInt32(rule, 0x124, 0xF);
        Marshal.WriteInt32(rule, 0x128, world.ToInt32());
        Marshal.WriteInt32(rule, 0x12C, -1);

        text = new StringBuilder();
        text.AppendFormat("{0}\n  guard at 0x3160F1: {1}\n", exe, BitConverter.ToString(file, 0x3160F1 - TextVa + TextFile, 6));
        State("start: team 0 has 5 tickets, team 1 has 1");
        Death(0, 2, 100000);
        State("player 2 (team 1) dies to player 0: its last ticket");
        Death(0, 2, 110000);
        State("player 2 dies again: out of the alive mask");
        Death(1, 3, 120000);
        State("player 3, the last of team 1, dies: the rule completes");
        // A client's world after the rollback: the death is not yet in it.
        Rollback(5, 0, 0xB);
        State("rollback to before that death (tickets and masks loaded)");
        Death(1, 3, 120000);
        State("the same death played again");
        Death(3, 0, 125000);
        State("a later death: player 0 (team 0) killed by player 3");
        return text.ToString();
    }
}
'@

# 0x310000 is not always free in a process (about one start in five here):
# each exe runs in a process of its own, started again while the address is
# taken.
if ($env:SRP_TDM_RULE_CHILD) {
    try { [Tdm]::Run($Exes[0]) }
    catch { if ($_.Exception.InnerException.Message -like "*is not free*") { exit 3 }; throw }
    exit 0
}
$self = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$env:SRP_TDM_RULE_CHILD = "1"
foreach ($exe in $Exes) {
    $path = (Resolve-Path $exe).Path
    for ($try = 1; ; $try++) {
        & $self -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath $path
        if ($LASTEXITCODE -ne 3) { break }
        if ($try -ge 20) { throw "0x310000 was taken in 20 processes" }
    }
}
