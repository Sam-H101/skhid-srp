// The inventory's right-click menu offers Buy for a modified item too: in
// InventoryList.showContextMenu, `!itemData.rank` (no modifications, and not a
// bundle item) becomes `!itemData.fromBundle` (not a bundle item). Buying goes
// by the item's dictionary id, so it buys a stock item, unmodified. One
// operand of one instruction changes; every other method stays byte for byte.
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
    private static final int GETPROPERTY = 0x66;
    private static final String CLASS = "survarium.controls.InventoryList";

    private static String name(ABC abc, int multiname) {
        Multiname m = abc.constants.getMultiname(multiname);
        return m.name_index > 0 ? abc.constants.getString(m.name_index) : null;
    }

    private static int u30(int value) {
        int bytes = 1;
        while ((value >>>= 7) != 0) bytes++;
        return bytes;
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        ABC abc = null;
        int block = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName(CLASS) >= 0) {
                if (abc != null) throw new IOException("Two " + CLASS + " classes");
                abc = swf.getAbcList().get(i).getABC();
                block = i;
            }
        if (abc == null) throw new IOException("Missing " + CLASS);
        MethodBody body = abc.findBodyByClassAndName("InventoryList", "showContextMenu");
        if (body == null) throw new IOException("Missing InventoryList.showContextMenu");
        List<byte[]> originals = new ArrayList<>();
        for (MethodBody b : abc.bodies) originals.add(b.getCodeBytes().clone());

        AVM2Code code = body.getCode();
        AVM2Instruction read = null;
        for (AVM2Instruction ins : code.code)
            if (ins.definition.instructionCode == GETPROPERTY && "rank".equals(name(abc, ins.operands[0]))) {
                if (read != null) throw new IOException("showContextMenu reads rank more than once");
                read = ins;
            }
        if (read == null) throw new IOException("showContextMenu does not read rank (already patched?)");
        // `fromBundle` as the block already names it, in rank's namespace.
        Multiname rank = abc.constants.getMultiname(read.operands[0]);
        int fromBundle = -1;
        for (int i = 1; i < abc.constants.getMultinameCount(); i++) {
            Multiname m = abc.constants.getMultiname(i);
            if (m.kind == rank.kind && m.namespace_index == rank.namespace_index
                && m.namespace_set_index == rank.namespace_set_index && "fromBundle".equals(name(abc, i))) {
                fromBundle = i;
                break;
            }
        }
        if (fromBundle < 0) throw new IOException("No fromBundle name like rank's in the block");
        // The same length, so no jump of the method moves.
        if (u30(fromBundle) != u30(read.operands[0])) throw new IOException("fromBundle's index is not as long as rank's");
        read.operands[0] = fromBundle;
        body.setCode(code);
        ((Tag) abc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[1]))) { swf.saveTo(out); }

        SWF saved;
        try (InputStream in = Files.newInputStream(Path.of(args[1]))) { saved = new SWF(in, true); }
        ABC savedAbc = saved.getAbcList().get(block).getABC();
        if (savedAbc.bodies.size() != originals.size()) throw new IOException("Method count changed");
        int index = abc.bodies.indexOf(body), changed = 0;
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), savedAbc.bodies.get(i).getCodeBytes())) {
                if (i != index) throw new IOException("Unrelated method changed " + i);
                changed++;
            }
        if (changed != 1) throw new IOException("showContextMenu did not change");
        System.out.println("Buy offered for modified items; one instruction changed");
    }
}
