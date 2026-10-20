// Compile a class's source over the class of that name a live lobby movie
// already has (class_update_026g.py: SrpRooms, put in by rooms_026g.py);
// nothing else changes. The methods of every other class and every other
// script's initializer are compared by name before and after, byte for
// byte, and every constant keeps its index and value: replacing a script
// may renumber the bodies of its block. args: in.swf source.as out.swf class
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

public class ReplaceClass {
    private static String CLASS;

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
            // Each script's initializer, by the names it defines.
            for (var script : abc.script_info) {
                if (script.deleted) continue;
                List<String> names = new ArrayList<>();
                for (Trait trait : script.traits.traits)
                    names.add(trait.getName(abc).getNameWithNamespace(new HashSet<>(), abc, abc.constants, false)
                        .toRawString());
                if (names.contains(CLASS)) continue;
                Collections.sort(names);
                put(code, abc, b + ":script " + names, script.init_index);
            }
        }
        return code;
    }

    // The constant pools: what unchanged code points at keeps its value
    // (new entries may follow).
    private static List<List<String>> poolsOf(SWF swf) {
        List<List<String>> pools = new ArrayList<>();
        for (int b = 0; b < swf.getAbcList().size(); b++) {
            var pool = swf.getAbcList().get(b).getABC().constants;
            List<String> entries = new ArrayList<>();
            for (int i = 1; i < pool.getStringCount(); i++) entries.add("s" + i + "=" + pool.getString(i));
            for (int i = 1; i < pool.getIntCount(); i++) entries.add("i" + i + "=" + pool.getInt(i));
            for (int i = 1; i < pool.getUIntCount(); i++) entries.add("u" + i + "=" + pool.getUInt(i));
            for (int i = 1; i < pool.getDoubleCount(); i++) entries.add("d" + i + "=" + pool.getDouble(i));
            for (int i = 1; i < pool.getNamespaceCount(); i++) {
                var ns = pool.getNamespace(i);
                entries.add("n" + i + "=" + ns.kind + "/" + ns.name_index);
            }
            for (int i = 1; i < pool.getNamespaceSetCount(); i++)
                entries.add("ns" + i + "=" + Arrays.toString(pool.getNamespaceSet(i).namespaces));
            for (int i = 1; i < pool.getMultinameCount(); i++) {
                var name = pool.getMultiname(i);
                entries.add("m" + i + "=" + name.kind + "/" + name.name_index + "/" + name.namespace_index
                    + "/" + name.namespace_set_index + "/" + name.qname_index);
            }
            pools.add(entries);
        }
        return pools;
    }

    private static ScriptPack pack(SWF swf) throws IOException {
        ScriptPack found = null;
        for (ScriptPack pack : swf.getAS3Packs())
            if (pack.getClassPath().toRawString().equals(CLASS)) {
                if (found != null) throw new IOException("Two " + CLASS + " scripts");
                found = pack;
            }
        if (found == null) throw new IOException("No " + CLASS + " in the movie: it has to be built in first");
        return found;
    }

    public static void main(String[] args) throws Exception {
        CLASS = args[3];
        SWF swf;
        try (InputStream in = Files.newInputStream(Path.of(args[0]))) { swf = new SWF(in, true); }
        Map<String, byte[]> before = codeOf(swf);
        List<List<String>> poolsBefore = poolsOf(swf);
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
        List<List<String>> poolsAfter = poolsOf(saved);
        for (int b = 0; b < poolsBefore.size(); b++)
            if (poolsAfter.get(b).size() < poolsBefore.get(b).size()
                || !new HashSet<>(poolsAfter.get(b)).containsAll(poolsBefore.get(b)))
                throw new IOException("A constant of block " + b + " changed");
        System.out.println(CLASS + " replaced; " + before.size() + " other methods and scripts unchanged, "
            + "every constant kept");
    }
}
