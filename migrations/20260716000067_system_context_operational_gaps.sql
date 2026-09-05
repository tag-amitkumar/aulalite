-- Complete the system-context policies for cross-tenant operational jobs.
--
-- `app.system` is transaction-local and is set only by trusted in-process
-- server operations through `db::begin_system_context`. Normal request paths
-- continue to use `app.tenant_id` and are unaffected.

-- The chat-retention janitor deletes expired messages across every tenant.
CREATE POLICY system_context_delete ON live_room_messages
    FOR DELETE
    USING (current_setting('app.system', true) = 'on');
