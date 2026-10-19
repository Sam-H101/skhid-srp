// Buy on an item the account owns (its right-click menu, its card's Buy):
//   InventoryList.showContextMenu  Buy was listed for an item that is not a
//       stack only when `!itemData.rank`; an owned item's rank is its number
//       of modifications. It is listed now when `!rank || itemAvailable` (the
//       method's own "owned and enabled"), so a modified owned item lists Buy;
//       shop, bundle and bundle-content entries keep their rule. Buying goes by
//       the dictionary id: it buys a stock item, unmodified.
//   LobbyMenu.try_buy_item  "This item is already purchased by you. Would you
//       like to buy anyway?" was asked for any buy of a non-stack item the
//       account has, but from a bundle (`sourceSlot != 300`); it is asked only
//       for a buy from the shop's list (`sourceSlot == 200`): `pushshort 300;
//       equals; not` becomes `pushshort 200; equals; nop`. A buy on an owned
//       item comes with that item's own slot (100 or less) and is not asked.
// Every other method stays byte for byte.
import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Multiname;
import com.jpexs.decompiler.flash.tags.Tag;

public class PatchBuyModified {
    private static final int GETPROPERTY = 0x66, PUSHSHORT = 0x25, EQUALS = 0xab, NOT = 0x96, NOP = 0x02,
        DEBUG = 0xef, CONVERT_B = 0x76, GETLOCAL = 0x62, BITAND = 0xa8;
    private static final int BUNDLE = 300, SHOP = 200;

    private static String name(ABC abc, int multiname) {
        Multiname m = abc.constants.getMultiname(multiname);
        return m.name_index > 0 ? abc.constants.getString(m.name_index) : null;
    }

    private static int u30(int value) {
        int bytes = 1;
        while ((value >>>= 7) != 0) bytes++;
        return bytes;
    }

    private static int blockOf(SWF swf, String cls) throws IOException {
        int found = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName(cls) >= 0) {
                if (found >= 0) throw new IOException("Two " + cls + " classes");
                found = i;
            }
        if (found < 0) throw new IOException("Missing " + cls);
        return found;
    }

    // FFDec finds a body by the class's own name: that name is the block's once.
    private static MethodBody body(ABC abc, String cls, String method) throws IOException {
        int named = 0;
        for (var info : abc.instance_info)
            if (info.getName(abc.constants).getName(new HashSet<>(), abc, abc.constants, new ArrayList<>(), false, false).equals(cls))
                named++;
        if (named != 1) throw new IOException(named + " classes named " + cls + " in the block");
        MethodBody body = abc.findBodyByClassAndName(cls, method);
        if (body == null) throw new IOException("Missing " + cls + "." + method);
        return body;
    }

    private static int code(AVM2Instruction ins) {
        return ins.definition.instructionCode;
    }

    // showContextMenu: `itemData.rank`, read once, becomes
    // `(itemData.rank ? 1 : 0) & (itemAvailable ? 0 : 1)`; the `not` after it
    // then gives `!rank || itemAvailable`. No jump is added.
    private static void menu(ABC abc, MethodBody body) throws Exception {
        AVM2Code code = body.getCode();
        int read = -1, available = -1;
        for (int i = 0; i < code.code.size(); i++) {
            AVM2Instruction ins = code.code.get(i);
            if (code(ins) == GETPROPERTY && "rank".equals(name(abc, ins.operands[0]))) {
                if (read >= 0) throw new IOException("showContextMenu reads rank more than once");
                read = i;
            }
            // The debug names count registers from the first after `this`.
            if (code(ins) == DEBUG && "itemAvailable".equals(abc.constants.getString(ins.operands[1])))
                available = ins.operands[2] + 1;
        }
        if (read < 0) throw new IOException("showContextMenu does not read rank (already patched?)");
        if (available < 0) throw new IOException("showContextMenu has no itemAvailable");
        if (code(code.code.get(read + 1)) != NOT) throw new IOException("showContextMenu does not test !rank");
        int[][] added = {{CONVERT_B}, {GETLOCAL, available}, {NOT}, {BITAND}};
        for (int k = 0; k < added.length; k++)
            code.insertInstruction(read + 1 + k, new AVM2Instruction(0, added[k][0],
                Arrays.copyOfRange(added[k], 1, added[k].length)), body);
        body.max_stack += 1;
        body.setCode(code);
        body.getCode().checkValidOffsets(body);
    }

    // try_buy_item: the one `sourceSlot != 300` becomes `sourceSlot == 200`.
    private static void ask(MethodBody body) throws Exception {
        AVM2Code code = body.getCode();
        int at = -1;
        for (int i = 0; i + 2 < code.code.size(); i++)
            if (code(code.code.get(i)) == PUSHSHORT && code.code.get(i).operands[0] == BUNDLE
                && code(code.code.get(i + 1)) == EQUALS && code(code.code.get(i + 2)) == NOT) {
                if (at >= 0) throw new IOException("try_buy_item tests sourceSlot != 300 more than once");
                at = i;
            }
        if (at < 0) throw new IOException("try_buy_item does not test sourceSlot != 300 (already patched?)");
        if (u30(SHOP) != u30(BUNDLE)) throw new IOException("200 is not as long as 300");
        code.code.get(at).operands[0] = SHOP;
        code.code.set(at + 2, new AVM2Instruction(code.code.get(at + 2).getAddress(), NOP, new int[]{}));
        body.setCode(code);
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        int blocks = swf.getAbcList().size();
        List<List<byte[]>> originals = new ArrayList<>();
        for (int i = 0; i < blocks; i++) {
            List<byte[]> code = new ArrayList<>();
            for (MethodBody b : swf.getAbcList().get(i).getABC().bodies) code.add(b.getCodeBytes().clone());
            originals.add(code);
        }
        int listBlock = blockOf(swf, "survarium.controls.InventoryList");
        int lobbyBlock = blockOf(swf, "LobbyMenu");
        ABC listAbc = swf.getAbcList().get(listBlock).getABC();
        ABC lobbyAbc = swf.getAbcList().get(lobbyBlock).getABC();
        MethodBody showMenu = body(listAbc, "InventoryList", "showContextMenu");
        MethodBody tryBuy = body(lobbyAbc, "LobbyMenu", "try_buy_item");
        int buyLength = tryBuy.getCodeBytes().length;
        menu(listAbc, showMenu);
        ask(tryBuy);
        if (tryBuy.getCodeBytes().length != buyLength) throw new IOException("try_buy_item changed length");
        ((Tag) listAbc.parentTag).setModified(true);
        ((Tag) lobbyAbc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[1]))) { swf.saveTo(out); }

        SWF saved;
        try (InputStream in = Files.newInputStream(Path.of(args[1]))) { saved = new SWF(in, true); }
        int changed = 0;
        for (int b = 0; b < blocks; b++) {
            List<MethodBody> bodies = saved.getAbcList().get(b).getABC().bodies;
            if (bodies.size() != originals.get(b).size()) throw new IOException("Method count changed in block " + b);
            for (int i = 0; i < bodies.size(); i++)
                if (!Arrays.equals(originals.get(b).get(i), bodies.get(i).getCodeBytes())) {
                    boolean expected = (b == listBlock && i == listAbc.bodies.indexOf(showMenu))
                        || (b == lobbyBlock && i == lobbyAbc.bodies.indexOf(tryBuy));
                    if (!expected) throw new IOException("Unrelated method changed: block " + b + " method " + i);
                    changed++;
                }
        }
        if (changed != 2) throw new IOException("Expected two methods changed, got " + changed);
        System.out.println("Buy on an owned item: listed for modified items, not asked again; two methods changed");
    }
}
