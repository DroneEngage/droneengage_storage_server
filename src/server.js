"use strict";

const v_pjson           = require('../package.json');
global.Colors           = require ("droneengage_server_common").helpers.colors.Colors;
global.m_serverconfig   = require ('../js_serverConfig.js');

let v_configFileName = global.m_serverconfig.getFileName();

const m_storage_server = require ('./index.js');

process.on('SIGINT', function() {
    if (global.m_logger) global.m_logger.Warn('SIGINT.');
    process.exit(0);
});


function checkMemory()
    {
        const used = process.memoryUsage();
        let readings = "";
        for (let key in used) {
            readings += `${key} ${Math.round(used[key] / 1024 / 1024 * 100) / 100} MB - `;
        }
        console.log(readings);

        // Check memory limit
        if (global.m_serverconfig.m_configuration.memory_max != null) {
            const mem = Math.round(used.rss / 1024 / 1024 * 100) / 100;
            if (global.m_serverconfig.m_configuration.memory_max < mem) {
                console.log("Memory is " + global.Colors.FgYellow + mem + global.Colors.Error + ' RESTART' + global.Colors.Reset);
                process.exit(1);
            }
        }
    }

function fn_displayHelp ()
{
    console.log ("==================================")
    console.log (global.Colors.Bright + "DroneEngage Storage Server version " +  JSON.stringify(v_pjson.version) + global.Colors.Reset);
    console.log ("----------------------------------");
    console.log ("--config=config_filename config file ");
    console.log ("-h help ");
    console.log ("-v version");
    console.log ("==================================");
}


function fn_displayInfo ()
{
    console.log ("=============================================")
    console.log (global.Colors.Bright + "DE Storage Server version " +  JSON.stringify(v_pjson.version) + global.Colors.Reset);
    console.log ("---------------------------------------------");
    console.log ("Server Name  " + global.Colors.BSuccess +  global.m_serverconfig.m_configuration.server_id + global.Colors.Reset);
    console.log("listening on ip: " + global.Colors.BSuccess +  global.m_serverconfig.m_configuration.server_ip + global.Colors.Reset + " port: " + global.Colors.BSuccess + (process.env.de_storage_server_port || global.m_serverconfig.m_configuration.server_port) + global.Colors.Reset);
    console.log ("Auth Server ip: " + global.Colors.BSuccess +  global.m_serverconfig.m_configuration.s2s_ws_target_ip + global.Colors.Reset + " port: " + global.Colors.BSuccess + global.m_serverconfig.m_configuration.s2s_ws_target_port + global.Colors.Reset);
    if (global.m_serverconfig.m_configuration.ignoreLog!==false)
    {
        console.log ("logging is " + global.Colors.FgYellow + 'disabled' + global.Colors.Reset);
    }
    else
    {
        global.m_logger         = require ('node-file-logger');

        const options = {
            timeZone: global.m_serverconfig.m_configuration.log_timeZone==null?'GMT':global.m_serverconfig.m_configuration.log_timeZone,      
            folderPath: global.m_serverconfig.m_configuration.log_directory==null?'./log':global.m_serverconfig.m_configuration.log_directory,      
            dateBasedFileNaming: true,
            fileName: 'All_Logs',   
            fileNamePrefix: 'Logs_',
            fileNameSuffix: '',
            fileNameExtension: '.log',     
            dateFormat: 'YYYY-MM-DD',
            timeFormat: 'HH:mm:ss.SSS',
            logLevel: global.m_serverconfig.m_configuration.log_detailed==true?'debug':'prod',
            onlyFileLogging: true
          };
        
        global.m_logger.SetUserOptions(options); 

        console.log ("logging is " + global.Colors.FgYellow + 'enabled' + global.Colors.Reset);

        if (global.m_logger) global.m_logger.Info('System Started.');
    }
    
    console.log ("Datetime: %s", new Date());
    console.log ("=====================================================================");
}




function fn_parseArgs()
{
    const c_args = require ('droneengage_server_common').helpers.args;

    var cmds = c_args.getArgs();
    if (cmds.hasOwnProperty('h') || cmds.hasOwnProperty('help'))
    {
        fn_displayHelp();
        process.exit(0);
    }

    if (cmds.hasOwnProperty('v') || cmds.hasOwnProperty('version'))
    {
        console.log ("DroneEngage Storage Server version: " + JSON.stringify(v_pjson.version));
        process.exit(0);
    }

    if (cmds.hasOwnProperty('config') )
    {
        v_configFileName = cmds.config;
    }
}




function fn_initSingletons()
{
    // Add singletons if needed
}

/**
 * Start Server
 */
function fn_startServer ()
{
    // checking memory
    setInterval(checkMemory, 60000);

    // parse input arguments
    fn_parseArgs();

    // Singletons init
    fn_initSingletons();

    // load server configuration
    global.m_serverconfig.init(v_configFileName);
        
    // display info
    fn_displayInfo();
    
    // Start storage server
    const server = new m_storage_server();
    server.start().catch(error => {
        console.log(global.Colors.Error + "Fatal error: " + error.message + global.Colors.Reset);
        process.exit(1);
    });
}


fn_startServer();
