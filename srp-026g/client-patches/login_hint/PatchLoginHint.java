// Compile the hint and change only the three LoginMenu entry points.
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

public class PatchLoginHint {
    private static void append(ABC abc, MethodBody body, int helper) throws Exception {
        AVM2Code code = body.getCode();
        int end = code.code.size() - 1;
        if (code.code.get(end).definition.instructionCode != 0x47 || body.exceptions.length != 0)
            throw new IOException("Unexpected LoginMenu method layout");
        code.insertInstruction(end++, new AVM2Instruction(0, 0x60, new int[]{helper}), body);
        code.insertInstruction(end++, new AVM2Instruction(0, 0xd0, new int[]{}), body);
        code.insertInstruction(end, new AVM2Instruction(0, 0x4f,
            new int[]{abc.constants.getPublicQnameId("apply", true), 1}), body);
        body.max_stack = Math.max(body.max_stack, 2);
        body.setCode(code);
        body.getCode().checkValidOffsets(body);
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        int abcIndex = -1;
        for (int i = 0; i < swf.getAbcList().size(); i++)
            if (swf.getAbcList().get(i).getABC().findClassByName("LoginMenu") >= 0) {
                if (abcIndex >= 0) throw new IOException("Multiple LoginMenu classes");
                abcIndex = i;
            }
        if (abcIndex < 0) throw new IOException("Missing LoginMenu class");
        ABC abc = swf.getAbcList().get(abcIndex).getABC();
        if (abc.findClassByName("SrpLoginHint") != -1) throw new IOException("Already patched");
        List<byte[]> originals = new ArrayList<>();
        for (MethodBody body : abc.bodies) originals.add(body.getCodeBytes().clone());
        swf.getAbcIndex().selectAbc(abc);
        ActionScript3Parser.compile(Files.readString(Path.of(args[1])), abc,
            swf.getAbcIndex(), "SrpLoginHint.as", abc.instance_info.size(),
            abc.script_info.size(), false, "");
        for (int i = 0; i < originals.size(); i++)
            if (!Arrays.equals(originals.get(i), abc.bodies.get(i).getCodeBytes()))
                throw new IOException("Compiler changed existing method " + i);
        int helper = abc.instance_info.get(abc.findClassByName("SrpLoginHint")).name_index;
        MethodBody ctor = abc.findBodyInstanceInitializerByClass("LoginMenu");
        MethodBody local = abc.findBodyByClassAndName("LoginMenu", "set_localization_data");
        MethodBody click = abc.findBodyByClassAndName("LoginMenu", "onRegistrationClicked");
        append(abc, ctor, helper);
        append(abc, local, helper);
        // Remove the browser callback as well as hiding/removing focus from the button.
        click.setCodeBytes(new byte[]{0x47});
        ((Tag)abc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[2]))) { swf.saveTo(out); }
        try (InputStream in = Files.newInputStream(Path.of(args[2]))) {
            ABC saved = new SWF(in, true).getAbcList().get(abcIndex).getABC();
            if (saved.findClassByName("SrpLoginHint") < 0) throw new IOException("Missing hint");
            for (int i = 0; i < abc.bodies.size(); i++)
                if (!Arrays.equals(abc.bodies.get(i).getCodeBytes(), saved.bodies.get(i).getCodeBytes()))
                    throw new IOException("Serialization changed method " + i);
            for (int i = 0; i < originals.size(); i++) {
                MethodBody body = abc.bodies.get(i);
                if (body != ctor && body != local && body != click
                    && !Arrays.equals(originals.get(i), saved.bodies.get(i).getCodeBytes()))
                    throw new IOException("Unrelated method changed " + i);
            }
        }
        System.out.println("Login hint compiled; all unrelated methods preserved");
    }
}
