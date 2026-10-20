// Compile SrpBuyGold into the exchange window's ABC block and make every way
// to the online store open it instead. The bodies replaced, each by
// `SrpBuyGold.open()`:
//   <class>.goToStore                 for each class in STORE: the "Get Gold"
//                                     of their "Insufficient Gold" dialogs
//   MessagePremium.onPremiumMoneyAddClick  the premium window's gold button,
//                                     which asked first whether to open a browser
// Afterwards no method of the movie pushes "go_to_store"; nothing else changes.
import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.avm2.parser.script.ActionScript3Parser;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.tags.Tag;

public class PatchBuyGold {
    private static final int GETLEX = 0x60, PUSHSTRING = 0x2c, CALLPROPVOID = 0x4f, RETURNVOID = 0x47;
    private static final String[] STORE = {
        "survarium.controls.ItemCard",
        "survarium.controls.StatusPanel",
        "survarium.messages.MessageAsk",
        "survarium.messages.MessageCraftItem",
        "survarium.messages.MessagePostPremium",
        "survarium.messages.MessagePremium",
        "survarium.messages.MessagePremiumDiscount",
        "survarium.messages.MessagePremiumProfile",
        "survarium.messages.MessageRepair",
        "survarium.messages.MessageRerollItem",
    };

    // FFDec finds bodies by the class's own name, without its package.
    private static MethodBody method(ABC abc, String cls, String name) throws IOException {
        MethodBody body = abc.findBodyByClassAndName(cls.substring(cls.lastIndexOf('.') + 1), name);
        if (body == null) throw new IOException("Missing " + cls + "." + name);
        return body;
    }

    private static int abcWith(SWF swf, String cls) throws IOException {
        int found = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName(cls) >= 0) {
                if (found >= 0) throw new IOException("Multiple " + cls + " classes");
                found = i;
            }
        if (found < 0) throw new IOException("Missing " + cls);
        return found;
    }

    private static boolean pushes(ABC abc, MethodBody body, String text) throws Exception {
        for (AVM2Instruction ins : body.getCode().code)
            if (ins.definition.instructionCode == PUSHSTRING
                && text.equals(abc.constants.getString(ins.operands[0])))
                return true;
        return false;
    }

    // The whole body becomes `SrpBuyGold.open()`.
    private static void replace(ABC abc, MethodBody body) throws Exception {
        body.exceptions = new com.jpexs.decompiler.flash.abc.types.ABCException[0];
        body.setCodeBytes(new byte[]{(byte) RETURNVOID});
        AVM2Code code = body.getCode();
        code.insertInstruction(0, new AVM2Instruction(0, GETLEX,
            new int[]{abc.constants.getPublicQnameId("SrpBuyGold", true)}), body);
        code.insertInstruction(1, new AVM2Instruction(0, CALLPROPVOID,
            new int[]{abc.constants.getPublicQnameId("open", true), 0}), body);
        body.max_stack = Math.max(body.max_stack, 1);
        body.setCode(code);
        body.getCode().checkValidOffsets(body);
    }

    private static List<byte[]> codeOf(ABC abc) {
        List<byte[]> code = new ArrayList<>();
        for (MethodBody body : abc.bodies) code.add(body.getCodeBytes().clone());
        return code;
    }

    // Every method of `saved` but `changed` is byte for byte `originals`;
    // `edited` (the patched ABC in memory) survived serialization.
    private static void verify(ABC edited, ABC saved, List<byte[]> originals, Set<MethodBody> changed)
            throws IOException {
        if (edited.bodies.size() != saved.bodies.size()) throw new IOException("Method count changed");
        for (int i = 0; i < edited.bodies.size(); i++)
            if (!Arrays.equals(edited.bodies.get(i).getCodeBytes(), saved.bodies.get(i).getCodeBytes()))
                throw new IOException("Serialization changed method " + i);
        for (int i = 0; i < originals.size(); i++)
            if (!changed.contains(edited.bodies.get(i))
                && !Arrays.equals(originals.get(i), saved.bodies.get(i).getCodeBytes()))
                throw new IOException("Unrelated method changed " + i);
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        int blocks = swf.getAbcList().size();
        for (int i = 0; i < blocks; i++)
            if (swf.getAbcList().get(i).getABC().findClassByName("SrpBuyGold") != -1)
                throw new IOException("Already patched");
        // The exchange window's block takes the new class; classes are found
        // by name at run time, and every block is loaded long before a click.
        int home = abcWith(swf, "SrpExchange");
        List<List<byte[]>> originals = new ArrayList<>();
        for (int i = 0; i < blocks; i++) originals.add(codeOf(swf.getAbcList().get(i).getABC()));

        // Find every body first: nothing changes if one is missing or unexpected.
        Map<MethodBody, ABC> targets = new LinkedHashMap<>();
        for (String cls : STORE) {
            ABC abc = swf.getAbcList().get(abcWith(swf, cls)).getABC();
            MethodBody body = method(abc, cls, "goToStore");
            if (!pushes(abc, body, "go_to_store")) throw new IOException(cls + ".goToStore does not open the store");
            targets.put(body, abc);
        }
        String premium = "survarium.messages.MessagePremium";
        ABC premiumAbc = swf.getAbcList().get(abcWith(swf, premium)).getABC();
        MethodBody plus = method(premiumAbc, premium, "onPremiumMoneyAddClick");
        if (!pushes(premiumAbc, plus, "go_to_online_store_message"))
            throw new IOException(premium + ".onPremiumMoneyAddClick does not ask for the store");
        targets.put(plus, premiumAbc);
        if (targets.size() != STORE.length + 1) throw new IOException("Two entry points share a body");

        ABC homeAbc = swf.getAbcList().get(home).getABC();
        swf.getAbcIndex().selectAbc(homeAbc);
        ActionScript3Parser.compile(Files.readString(Path.of(args[1])), homeAbc,
            swf.getAbcIndex(), "SrpBuyGold.as", homeAbc.instance_info.size(),
            homeAbc.script_info.size(), false, "");
        List<byte[]> homeOriginals = originals.get(home);
        for (int i = 0; i < homeOriginals.size(); i++)
            if (!Arrays.equals(homeOriginals.get(i), homeAbc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);

        for (Map.Entry<MethodBody, ABC> target : targets.entrySet()) {
            replace(target.getValue(), target.getKey());
            ((Tag) target.getValue().parentTag).setModified(true);
        }
        ((Tag) homeAbc.parentTag).setModified(true);
        for (int i = 0; i < blocks; i++) {
            ABC abc = swf.getAbcList().get(i).getABC();
            for (MethodBody body : abc.bodies)
                if (pushes(abc, body, "go_to_store"))
                    throw new IOException("A way to the store is left in block " + i);
        }
        try (OutputStream out = Files.newOutputStream(Path.of(args[2]))) { swf.saveTo(out); }

        try (InputStream in = Files.newInputStream(Path.of(args[2]))) {
            SWF saved = new SWF(in, true);
            if (saved.getAbcList().size() != blocks) throw new IOException("ABC block count changed");
            if (saved.getAbcList().get(home).getABC().findClassByName("SrpBuyGold") < 0)
                throw new IOException("Missing SrpBuyGold");
            Set<MethodBody> changed = new HashSet<>(targets.keySet());
            for (int i = 0; i < blocks; i++) {
                ABC abc = saved.getAbcList().get(i).getABC();
                verify(swf.getAbcList().get(i).getABC(), abc, originals.get(i), changed);
                for (MethodBody body : abc.bodies)
                    if (pushes(abc, body, "go_to_store"))
                        throw new IOException("The saved movie still opens the store in block " + i);
            }
        }
        System.out.println("Buy gold compiled; " + targets.size() + " ways to the store open the exchange");
    }
}
