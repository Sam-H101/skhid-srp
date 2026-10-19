// Compile exchange/SrpExchange.as over the SrpExchange class a live lobby movie
// already has (exchange_026g.py put it there); nothing else changes. The
// methods of every other class are compared by name before and after, byte
// for byte: replacing a script may renumber the bodies of its block.
import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.ScriptPack;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.Trait;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import com.jpexs.decompiler.flash.abc.types.traits.Traits;
import com.jpexs.decompiler.flash.importers.As3ScriptReplacerFactory;
import com.jpexs.decompiler.flash.tags.Tag;

public class ReplaceExchange {
    private static final String CLASS = "SrpExchange";

    private static void put(Map<String, byte[]> code, ABC abc, String key, int method) throws IOException {
        MethodBody body = abc.findBody(method);
        if (body != null && code.put(key, body.getCodeBytes().clone()) != null)
            throw new IOException("Two methods named " + key);
    }

    private static void traits(Map<String, byte[]> code, ABC abc, String owner, Traits traits) throws IOException {
        for (Trait trait : traits.traits)
            if (trait instanceof TraitMethodGetterSetter)
                put(code, abc, owner + trait.getName(abc)
                    .getNameWithNamespace(new HashSet<>(), abc, abc.constants, false).toRawString()
                    + "#" + trait.kindType, ((TraitMethodGetterSetter) trait).method_info);
    }

    // Every class's methods but SrpExchange's, by block, class and name.
    private static Map<String, byte[]> codeOf(SWF swf) throws IOException {
        Map<String, byte[]> code = new TreeMap<>();
        for (int b = 0; b < swf.getAbcList().size(); b++) {
            ABC abc = swf.getAbcList().get(b).getABC();
            for (int c = 0; c < abc.instance_info.size(); c++) {
                String name = abc.instance_info.get(c).getName(abc.constants)
                    .getNameWithNamespace(new HashSet<>(), abc, abc.constants, false).toRawString();
                if (name.equals(CLASS)) continue;
                String owner = b + ":" + name;
                put(code, abc, owner + "<init>", abc.instance_info.get(c).iinit_index);
                put(code, abc, owner + "<clinit>", abc.class_info.get(c).cinit_index);
                traits(code, abc, owner + ".", abc.instance_info.get(c).instance_traits);
                traits(code, abc, owner + ".static ", abc.class_info.get(c).static_traits);
            }
        }
        return code;
    }

    private static ScriptPack pack(SWF swf) throws IOException {
        ScriptPack found = null;
        for (ScriptPack pack : swf.getAS3Packs())
            if (pack.getClassPath().toRawString().equals(CLASS)) {
                if (found != null) throw new IOException("Two " + CLASS + " scripts");
                found = pack;
            }
        if (found == null) throw new IOException("No " + CLASS + ": build the exchange first (exchange_026g.py)");
        return found;
    }

    public static void main(String[] args) throws Exception {
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        Map<String, byte[]> before = codeOf(swf);
        ScriptPack pack = pack(swf);
        if (!pack.abc.replaceScriptPack(As3ScriptReplacerFactory.createFFDec(), pack,
                Files.readString(Path.of(args[1])), new ArrayList<SWF>()))
            throw new IOException("FFDec did not replace " + CLASS);
        ((Tag) pack.abc.parentTag).setModified(true);
        try (OutputStream out = Files.newOutputStream(Path.of(args[2]))) { swf.saveTo(out); }

        SWF saved;
        try (InputStream in = Files.newInputStream(Path.of(args[2]))) { saved = new SWF(in, true); }
        pack(saved);
        Map<String, byte[]> after = codeOf(saved);
        if (!before.keySet().equals(after.keySet())) throw new IOException("The other classes' methods changed");
        for (String key : before.keySet())
            if (!Arrays.equals(before.get(key), after.get(key)))
                throw new IOException("Unrelated method changed: " + key);
        System.out.println(CLASS + " replaced; " + before.size() + " other methods unchanged");
    }
}
