import io.waylingo.*
import kotlin.test.Test
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

/** Checks that need no network: the library loads, and bad input is rejected. */
class OfflineTest {
    private val config = WaylingoConfig(
        mapProvider = MapProvider.TOMTOM,
        llm = LlmBackend.Anthropic(model = null, baseUrl = null),
    )

    @Test
    fun theLibraryLoads() {
        assertTrue(version().isNotEmpty())
    }

    @Test
    fun anEmptyMapKeyIsRejected() {
        assertFailsWith<WaylingoException.InvalidConfig> { Waylingo(config, "", "llm-key") }
    }

    @Test
    fun aMissingLlmKeyIsRejected() {
        assertFailsWith<WaylingoException.InvalidConfig> { Waylingo(config, "map-key") }
    }

    @Test
    fun aResultThatIsNotWaylingosIsRejected() {
        val app = Waylingo(config, "map-key", "llm-key")
        assertFailsWith<WaylingoException.IncompatibleResult> {
            app.rerouteBlocking("{}", Choices())
        }
    }
}
