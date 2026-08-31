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
    enableHashHandling: false   // dashboard loads $$HASH$$ lazily via js_router_admin
});
