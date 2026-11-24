package gg.hypolia.velocity;

import com.velocitypowered.api.permission.Tristate;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class AccessPolicyTest {

    @Test
    void staffRanksAreAdminModAndDev() {
        assertTrue(AccessPolicy.isStaff("admin"));
        assertTrue(AccessPolicy.isStaff("mod"));
        assertTrue(AccessPolicy.isStaff("dev"));
        assertFalse(AccessPolicy.isStaff("vip"));
        assertFalse(AccessPolicy.isStaff("member"));
        assertFalse(AccessPolicy.isStaff("guest"));
        assertFalse(AccessPolicy.isStaff(null));
    }

    @Test
    void playersCannotUseProxyCommands() {
        assertEquals(Tristate.FALSE, AccessPolicy.permission("velocity.command.server", false));
        assertEquals(Tristate.FALSE, AccessPolicy.permission("velocity.command.send", false));
        assertEquals(Tristate.FALSE, AccessPolicy.permission("velocity.command.info", false));
    }

    @Test
    void staffCanUseProxyCommands() {
        assertEquals(Tristate.TRUE, AccessPolicy.permission("velocity.command.server", true));
        assertEquals(Tristate.TRUE, AccessPolicy.permission("velocity.command.glist", true));
    }

    @Test
    void otherPermissionsKeepTheirDefault() {
        assertEquals(Tristate.UNDEFINED, AccessPolicy.permission("hypolia.hub", false));
        assertEquals(Tristate.UNDEFINED, AccessPolicy.permission(null, true));
    }

    @Test
    void playersReachGameServersOnlyThroughTheOrchestrator() {
        assertFalse(AccessPolicy.mayConnect(false, false, false));
        assertTrue(AccessPolicy.mayConnect(false, false, true));
        assertTrue(AccessPolicy.mayConnect(false, true, false));
        assertTrue(AccessPolicy.mayConnect(true, false, false));
    }
}
