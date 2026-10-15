// Compile the exchange window and change only the five methods that lead to it.
// Two ABC blocks change: the window's (the new class, three methods) and the
// status panel's (its gold and silver buttons).
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

public class PatchExchange {
    private static final int GETLEX = 0x60, GETLOCAL0 = 0xd0, PUSHTRUE = 0x26,
        PUSHFALSE = 0x27, CALLPROPVOID = 0x4f, RETURNVOID = 0x47;

    // Inserts `SrpExchange.<name>(<argument>)` before the final returnvoid.
    private static void call(ABC abc, MethodBody body, int helper, String name, int argument)
            throws Exception {
        AVM2Code code = body.getCode();
        int end = code.code.size() - 1;
        if (code.code.get(end).definition.instructionCode != RETURNVOID || body.exceptions.length != 0)
            throw new IOException("Unexpected method layout before " + name);
        code.insertInstruction(end++, new AVM2Instruction(0, GETLEX, new int[]{helper}), body);
        code.insertInstruction(end++, new AVM2Instruction(0, argument, new int[]{}), body);
        code.insertInstruction(end, new AVM2Instruction(0, CALLPROPVOID,
            new int[]{abc.constants.getPublicQnameId(name, true), 1}), body);
        body.max_stack = Math.max(body.max_stack, 2);
        body.setCode(code);
        body.getCode().checkValidOffsets(body);
    }

    // The whole body becomes `SrpExchange.<name>(<argument>)`.
    private static void replace(ABC abc, MethodBody body, int helper, String name, int argument)
            throws Exception {
        body.exceptions = new com.jpexs.decompiler.flash.abc.types.ABCException[0];
        body.setCodeBytes(new byte[]{(byte) RETURNVOID});
        call(abc, body, helper, name, argument);
    }

    // FFDec finds bodies by the class's own name, without its package.
    private static MethodBody method(ABC abc, String cls, String name) throws IOException {
        String simple = cls.substring(cls.lastIndexOf('.') + 1);
        MethodBody body = name == null ? abc.findBodyInstanceInitializerByClass(simple)
                                       : abc.findBodyByClassAndName(simple, name);
        if (body == null) throw new IOException("Missing " + cls + "." + (name == null ? "<init>" : name));
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
        String window = "survarium.messages.MessageMoneyExchange";
        String panel = "survarium.controls.StatusPanel";
        // The window and the status panel are in two ABC blocks; the new class
        // goes into the window's. Classes are found by name at run time, and
        // both blocks are loaded long before a click.
        int windowIndex = abcWith(swf, window), panelIndex = abcWith(swf, panel);
        if (windowIndex == panelIndex) throw new IOException("Expected two ABC blocks");
        ABC abc = swf.getAbcList().get(windowIndex).getABC();
        ABC panelAbc = swf.getAbcList().get(panelIndex).getABC();
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName("SrpExchange") != -1)
                throw new IOException("Already patched");
        List<byte[]> originals = codeOf(abc), panelOriginals = codeOf(panelAbc);
        swf.getAbcIndex().selectAbc(abc);
        ActionScript3Parser.compile(Files.readString(Path.of(args[1])), abc,
            swf.getAbcIndex(), "SrpExchange.as", abc.instance_info.size(),
            abc.script_info.size(), false, "");
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), abc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);
        int helper = abc.instance_info.get(abc.findClassByName("SrpExchange")).name_index;
        int panelHelper = panelAbc.constants.getPublicQnameId("SrpExchange", true);

        MethodBody ctor = method(abc, window, null);
        MethodBody configUI = method(abc, window, "configUI");
        MethodBody accept = method(abc, window, "onMessageAccept");
        MethodBody gold = method(panelAbc, panel, "onPremiumMoneyAddClick");
        MethodBody silver = method(panelAbc, panel, "onRegulatMoneyAddClick");
        Set<MethodBody> changed = new HashSet<>(List.of(ctor, configUI, accept));
        Set<MethodBody> panelChanged = new HashSet<>(List.of(gold, silver));
        if (changed.size() != 3 || panelChanged.size() != 2)
            throw new IOException("Two entry points share a body");

        call(abc, ctor, helper, "build", GETLOCAL0);
        call(abc, configUI, helper, "apply", GETLOCAL0);
        replace(abc, accept, helper, "accept", GETLOCAL0);
        // The gold button asked to open the online store; the silver button
        // opened the stock window. Both open the exchange, each its own way.
        replace(panelAbc, gold, panelHelper, "open", PUSHTRUE);
        replace(panelAbc, silver, panelHelper, "open", PUSHFALSE);
        ((Tag) abc.parentTag).setModified(true);
        ((Tag) panelAbc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[2]))) { swf.saveTo(out); }

        try (InputStream in = Files.newInputStream(Path.of(args[2]))) {
            SWF saved = new SWF(in, true);
            ABC savedAbc = saved.getAbcList().get(windowIndex).getABC();
            if (savedAbc.findClassByName("SrpExchange") < 0) throw new IOException("Missing SrpExchange");
            verify(abc, savedAbc, originals, changed);
            verify(panelAbc, saved.getAbcList().get(panelIndex).getABC(), panelOriginals, panelChanged);
        }
        System.out.println("Exchange compiled; all unrelated methods preserved");
    }
}
