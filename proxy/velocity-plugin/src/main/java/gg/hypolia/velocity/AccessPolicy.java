package gg.hypolia.velocity;

import com.velocitypowered.api.permission.Tristate;

import java.util.Set;

final class AccessPolicy {

    private static final Set<String> STAFF_RANKS = Set.of("admin", "mod", "dev");
    private static final String PROXY_COMMANDS = "velocity.command.";

    private AccessPolicy() {
    }

    static boolean isStaff(String rank) {
        return rank != null && STAFF_RANKS.contains(rank);
    }

    static Tristate permission(String permission, boolean staff) {
        if (permission == null || !permission.startsWith(PROXY_COMMANDS)) {
            return Tristate.UNDEFINED;
        }
        return staff ? Tristate.TRUE : Tristate.FALSE;
    }

    static boolean mayConnect(boolean staff, boolean targetIsLobby, boolean sentByOrchestrator) {
        return staff || targetIsLobby || sentByOrchestrator;
    }
}
