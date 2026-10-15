// Compile the slider helper; call apply first and finish last in Settings.create_content.
import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.avm2.parser.script.ActionScript3Parser;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.Trait;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import com.jpexs.decompiler.flash.tags.Tag;

public class PatchFovSlider {
    private static final String SETTINGS = "survarium.menu.Settings";

    // helper.name(this) at index at.
    private static void call(ABC abc, AVM2Code code, MethodBody body, int at, int helper, String name) {
        code.insertInstruction(at++, new AVM2Instruction(0, 0x60, new int[]{helper}), body);
        code.insertInstruction(at++, new AVM2Instruction(0, 0xd0, new int[]{}), body);
        code.insertInstruction(at, new AVM2Instruction(0, 0x4f,
            new int[]{abc.constants.getPublicQnameId(name, true), 1}), body);
    }

    private static void wrap(ABC abc, MethodBody body, int helper) throws Exception {
        AVM2Code code = body.getCode();
        // getlocal0; pushscope opens every method body; apply goes right after
        // it. finish goes before the final returnvoid, after the options are
        // bound (the early return for missing content skips it).
        int end = code.code.size() - 1;
        if (code.code.size() < 3 || code.code.get(0).definition.instructionCode != 0xd0
            || code.code.get(1).definition.instructionCode != 0x30
            || code.code.get(end).definition.instructionCode != 0x47 || body.exceptions.length != 0)
            throw new IOException("Unexpected Settings.create_content layout");
        call(abc, code, body, end, helper, "finish");
        call(abc, code, body, 2, helper, "apply");
        body.max_stack = Math.max(body.max_stack, 2);
        body.setCode(code);
        body.getCode().checkValidOffsets(body);
    }

    // findBodyByClassAndName skips the protected namespace create_content uses.
    private static MethodBody method(ABC abc, String className, String name) throws IOException {
        MethodBody found = null;
        for (Trait trait : abc.instance_info.get(abc.findClassByName(className)).instance_traits.traits)
            if (trait instanceof TraitMethodGetterSetter && trait.kindType == Trait.TRAIT_METHOD
                && name.equals(abc.constants.getString(trait.getName(abc).name_index))) {
                if (found != null) throw new IOException("Multiple " + className + "." + name);
                found = abc.findBody(((TraitMethodGetterSetter) trait).method_info);
            }
        if (found == null) throw new IOException("Missing " + className + "." + name);
        return found;
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        int abcIndex = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName(SETTINGS) >= 0) {
                if (abcIndex >= 0) throw new IOException("Multiple Settings classes");
                abcIndex = i;
            }
        if (abcIndex < 0) throw new IOException("Missing Settings class");
        ABC abc = swf.getAbcList().get(abcIndex).getABC();
        if (abc.findClassByName("SrpFovSlider") != -1) throw new IOException("Already patched");
        List<byte[]> originals = new ArrayList<>();
        for (MethodBody body : abc.bodies) originals.add(body.getCodeBytes().clone());
        swf.getAbcIndex().selectAbc(abc);
        ActionScript3Parser.compile(Files.readString(Path.of(args[1])), abc,
            swf.getAbcIndex(), "SrpFovSlider.as", abc.instance_info.size(),
            abc.script_info.size(), false, "");
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), abc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);
        int helper = abc.instance_info.get(abc.findClassByName("SrpFovSlider")).name_index;
        MethodBody create = method(abc, SETTINGS, "create_content");
        wrap(abc, create, helper);
        ((Tag)abc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[2]))) { swf.saveTo(out); }
        try (InputStream in = Files.newInputStream(Path.of(args[2]))) {
            ABC saved = new SWF(in, true).getAbcList().get(abcIndex).getABC();
            if (saved.findClassByName("SrpFovSlider") < 0) throw new IOException("Missing slider helper");
            for (int i = 0; i < abc.bodies.size(); i++)
                if (!Arrays.equals(abc.bodies.get(i).getCodeBytes(), saved.bodies.get(i).getCodeBytes()))
                    throw new IOException("Serialization changed method " + i);
            for (int i = 0; i < originals.size(); i++)
                if (abc.bodies.get(i) != create && !Arrays.equals(originals.get(i), saved.bodies.get(i).getCodeBytes()))
                    throw new IOException("Unrelated method changed " + i);
        }
        System.out.println("FOV slider compiled; all unrelated methods preserved");
    }
}
