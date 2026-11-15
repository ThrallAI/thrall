package app.thrall.protocol

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolTest {
    /** Every `type` the engine's schema has for one kind of frame. */
    private fun schemaTypes(def: String): Set<String> {
        val path = System.getProperty("thrall.schema") ?: error("thrall.schema isn't set")
        val schema = Json.parseToJsonElement(File(path).readText()).jsonObject
        val variants = schema["\$defs"]!!.jsonObject[def]!!.jsonObject["oneOf"]!!.jsonArray
        return variants.mapNotNull {
            ((it as JsonObject)["properties"] as? JsonObject)?.get("type")?.jsonObject?.get("const")?.jsonPrimitive?.content
        }.toSet()
    }

    private fun noDrift(kind: String, schema: Set<String>, used: Set<String>, later: Set<String>) {
        assertTrue("$kind: the schema lists none", schema.isNotEmpty())
        val unknown = schema - used - later
        assertTrue("$kind the app doesn't know (use them or add them to ProtocolTypes): $unknown", unknown.isEmpty())
        val gone = (used + later) - schema
        assertTrue("$kind the engine no longer has: $gone", gone.isEmpty())
    }

    @Test
    fun theAppKnowsEveryTypeTheEngineHas() {
        noDrift("requests", schemaTypes("RequestFrame"), ProtocolTypes.requestsUsed, ProtocolTypes.requestsLater)
        noDrift("replies", schemaTypes("ReplyFrame"), ProtocolTypes.repliesUsed, ProtocolTypes.repliesLater)
        noDrift("events", schemaTypes("EventFrame"), ProtocolTypes.eventsUsed, ProtocolTypes.eventsLater)
    }

    @Test
    fun requestsAreFlatFrames() {
        val text = Protocol.frame("c-1", Request.TurnSend("s_1", "hi"))
        val obj = Json.parseToJsonElement(text).jsonObject
        assertEquals("1", obj["v"]!!.jsonPrimitive.content)
        assertEquals("c-1", obj["id"]!!.jsonPrimitive.content)
        assertEquals("turn.send", obj["type"]!!.jsonPrimitive.content)
        assertEquals("s_1", obj["session"]!!.jsonPrimitive.content)
        assertEquals(
            """{"v":1,"id":"p","type":"ping"}""",
            Protocol.frame("p", Request.Ping),
        )
        // The engine requires `token` in hello, even empty (the phone link ignores it).
        val hello = Json.parseToJsonElement(
            Protocol.frame("h", Request.Hello(listOf(1), ClientInfo("android", "Thrall", "0"))),
        ).jsonObject
        assertEquals("", hello["token"]!!.jsonPrimitive.content)
    }

    @Test
    fun answersMatchTheSpecsFrames() {
        // docs/protocol.md §15.
        assertEquals(
            """{"v":1,"id":"c-5","type":"permission.answer","request":"r_2","choice":"yes"}""",
            Protocol.frame("c-5", Request.PermissionAnswer("r_2", "yes")),
        )
        assertEquals(
            """{"v":1,"id":"c-7","type":"plan.decide","plan":"p_1","approve":true,"auto_edits":true}""",
            Protocol.frame("c-7", Request.PlanDecide("p_1", approve = true, autoEdits = true)),
        )
        assertEquals(
            """{"v":1,"id":"f","type":"folder.list","session":"s_1","path":""}""",
            Protocol.frame("f", Request.FolderList("s_1")),
        )
        assertEquals(
            """{"v":1,"id":"g","type":"folder.read","session":"s_1","path":"out/a.md","offset":0}""",
            Protocol.frame("g", Request.FolderRead("s_1", "out/a.md")),
        )
        assertEquals(
            """{"v":1,"id":"h","type":"step.file.read","session":"s_1","step":"p_3","index":0,"offset":0}""",
            Protocol.frame("h", Request.StepFileRead("s_1", "p_3", 0)),
        )
        val wrote = Protocol.parse("""{"v":1,"session":"s_1","seq":47,"at":0,"type":"step.completed","step":"p_3","status":"ok","summary":"Saved","saved":[{"path":"D:\\out\\a.pdf","name":"a.pdf","size":10}]}""") as Incoming.EventFrame
        assertEquals("a.pdf", (wrote.event as Event.StepCompleted).saved.single().name)
        val listed = Protocol.parse("""{"v":1,"re":"f","type":"folder.listed","session":"s_1","path":"","items":[{"name":"out","folder":true,"size":0,"modified":5},{"name":"a.md","size":8,"modified":4}]}""") as Incoming.ReplyFrame
        val items = (listed.reply as Reply.FolderListed).items
        assertEquals(listOf(true, false), items.map { it.folder })
        // auto_edits is always sent, even when off.
        assertTrue(Protocol.frame("c-8", Request.PlanDecide("p_1", approve = false)).contains(""""auto_edits":false"""))
        val ask = Protocol.parse("""{"v":1,"session":"s_1","seq":46,"at":0,"type":"permission.request","request":"r_2","turn":"t_9","step":"p_3","tool":"fs_delete","summary":"Delete build/ (214 files)","risk":"medium","choices":["yes","no","always"]}""") as Incoming.EventFrame
        assertEquals(listOf("yes", "no", "always"), (ask.event as Event.PermissionRequest).choices)
    }

    @Test
    fun theSpecsExamplesParse() {
        // Frames from docs/protocol.md §15.
        val welcome = Protocol.parse("""{"v":1,"re":"c-1","type":"welcome","protocol":1,"engine":{"version":"0.1.0","model":"x/y"},"user":{"id":"u","name":"Owner"},"capabilities":["streaming"],"limits":{"frame_bytes":1048576}}""")
        assertTrue(welcome is Incoming.ReplyFrame && welcome.reply is Reply.Welcome)
        val opened = Protocol.parse("""{"v":1,"re":"c-6","type":"session.opened","session":"s_1","title":"t","mode":"ask","state":"idle","last_seq":49,"updated":0,"cost_usd":0.0031}""") as Incoming.ReplyFrame
        assertEquals("s_1", (opened.reply as Reply.SessionOpened).info.session)
        val delta = Protocol.parse("""{"v":1,"session":"s_1","at":0,"type":"message.delta","message":"m_20","text":"I'll remove build/."}""") as Incoming.EventFrame
        assertEquals(Event.MessageDelta("m_20", "I'll remove build/."), delta.event)
        assertEquals(null, delta.seq)
        val done = Protocol.parse("""{"v":1,"session":"s_1","seq":49,"at":0,"type":"turn.completed","turn":"t_9","status":"done","usage":{"input_tokens":1840,"output_tokens":96,"cost_usd":0.0031}}""") as Incoming.EventFrame
        assertEquals(49L, done.seq)
        assertTrue(done.event is Event.TurnCompleted)
    }

    @Test
    fun thePhoneKnowsWhoItsFor() {
        // A member's phone: the engine says who; an engine too old to say a role means the owner.
        val sis = Protocol.parse("""{"v":1,"re":"h","type":"welcome","protocol":1,"engine":{"version":"0.1.0"},"user":{"id":"pe_1","name":"Sis","role":"member"},"limits":{"frame_bytes":1}}""") as Incoming.ReplyFrame
        val user = (sis.reply as Reply.Welcome).user
        assertEquals("Sis", user.name)
        assertFalse(user.owner)
        assertTrue(UserInfo("u", "Owner").owner)
        assertTrue(UserInfo("u", "Owner", "owner").owner)
        // Her own entry: the key is never sent, only whether it's set.
        val listed = Protocol.parse("""{"v":1,"re":"p","type":"people.listed","items":[{"person":"pe_1","name":"Sis","role":"member","added":1,"phones":1,"spent":0.42,"own_key":true,"key_set":false,"monthly_limit":5.0}]}""") as Incoming.ReplyFrame
        val me = (listed.reply as Reply.PeopleListed).items.single()
        assertEquals(PersonInfo("pe_1", "Sis", "member", 0.42, ownKey = true, keySet = false, monthlyLimit = 5.0), me)
        // Setting a key, and forgetting it (no key sent).
        assertEquals(
            """{"v":1,"id":"k","type":"people.key","key":"sk-or-1"}""",
            Protocol.frame("k", Request.PeopleKey("sk-or-1")),
        )
        assertEquals("""{"v":1,"id":"f","type":"people.key"}""", Protocol.frame("f", Request.PeopleKey(null)))
        assertEquals("""{"v":1,"id":"l","type":"people.list","since":7}""", Protocol.frame("l", Request.PeopleList(7)))
    }

    @Test
    fun whatTheAppDoesntKnowIsLeftAlone() {
        val future = Protocol.parse("""{"v":1,"session":"s","seq":3,"at":0,"type":"hologram.shown","x":1}""") as Incoming.EventFrame
        assertEquals(Event.Unknown, future.event)
        val reply = Protocol.parse("""{"v":1,"re":"c","type":"hologram.listed","items":[]}""") as Incoming.ReplyFrame
        assertEquals(Reply.Unknown, reply.reply)
        assertEquals(null, Protocol.parse("not json"))
    }
}
