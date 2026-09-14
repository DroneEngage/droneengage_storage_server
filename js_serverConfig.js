/*************************************************************************************
 * 
 *   S T O R A G E  S E R V E R -  Configuration File      JAVASCRIPT  LIB
 * 
 *   Author: Mohammad S. Hefny
 * 
 *   Date:   20 Jul 2026
 * 
 * 
 * 
 */

"use strict";

const common = require("droneengage_server_common");
const path = require("path");

module.exports = common.create({
    configDir: __dirname,
    enableHashHandling: false,   // dashboard loads $$HASH$$ lazily via js_router_admin
    envOverrides: {
        'de_storage_debug_logging': (cfg, val) => {
            cfg.debug_logging = (val === 'true' || val === '1');
        },
        // Per-account quota knobs (see server.config "limits" block).
        // Setting the env var overrides the file value; absent → file value wins.
        'de_storage_max_news_per_account': (cfg, val) => {
            if (val) { cfg.limits = cfg.limits || {}; cfg.limits.max_news_per_account = parseInt(val); }
        },
        'de_storage_max_missions_per_account': (cfg, val) => {
            if (val) { cfg.limits = cfg.limits || {}; cfg.limits.max_missions_per_account = parseInt(val); }
        },
        'de_storage_max_missions_per_unit': (cfg, val) => {
            if (val) { cfg.limits = cfg.limits || {}; cfg.limits.max_missions_per_unit = parseInt(val); }
        }
    }
});
