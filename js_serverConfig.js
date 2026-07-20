/*************************************************************************************
 * 
 *   S T O R A G E  S E R V E R -  Configuration File      JAVASCRIPT  LIB
 * 
 *   Author: DroneEngage Team
 * 
 *   Date:   20 Jul 2026
 * 
 * 
 * 
 */

"use strict";

const c_commentStripper = require("./helpers/js_3rd_StripJsonComments.js");
const c_configFileName_default = "server.config";

var Me = this;
exports.m_configuration = null;
var m_configFileName = c_configFileName_default;

exports.getFileName = function ()
{
    return m_configFileName;
}

exports.init = function init (p_configFileName)
{
    const  path = require('path');
    const fs = require('fs');

    if (p_configFileName != null)
    {
        m_configFileName = p_configFileName;
    }
        
    try
    {
        var v_filestring = fs.readFileSync(path.join(__dirname,m_configFileName)).toString();			
    }
    catch (err)
    {
        console.log ('FATAL: could not find ' + m_configFileName);
        console.log (err);
        process.exit(1);
    }

    try
    {
        Me.m_configuration = JSON.parse(c_commentStripper(v_filestring));
    }
    catch (err)
    {
        console.log ('FATAL: Bad File Format ' + m_configFileName);
        console.log (err);
        process.exit(1);
    }
}
