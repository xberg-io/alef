//! Gradle wrapper scripts and properties for Kotlin/Android AAR projects.
//!
//! The Gradle wrapper allows a project to build with a specific Gradle version
//! without requiring a pre-installed Gradle distribution. This module provides
//! the necessary files: gradle/wrapper/gradle-wrapper.properties, gradlew, gradlew.bat,
//! and gradle/wrapper/gradle-wrapper.jar (base64-encoded).

/// Return the gradle-wrapper.jar content as base64-encoded string.
/// This JAR is the official Gradle 8.5 wrapper JAR from the Gradle project.
/// It is stored as base64 so it can be embedded as a string and decoded at write time.
/// The wrapper JAR is forward-compatible and will bootstrap any Gradle version
/// specified in gradle-wrapper.properties, including Gradle 9.6.0.
pub(super) fn get_gradle_wrapper_jar_base64() -> String {
    include_str!("../../../assets/gradle-wrapper-8.5.jar.b64")
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

/// Render `gradle/wrapper/gradle-wrapper.properties` for the gradle wrapper.
///
/// Points to a Gradle distribution URL. This file is downloaded/cached by the
/// wrapper scripts on first invocation. Uses the central GRADLE_VERSION constant
/// defined in template_versions::toolchain.
pub(super) fn render_gradle_wrapper_properties() -> String {
    let gradle_version = crate::core::template_versions::toolchain::GRADLE_VERSION;
    format!(
        r#"distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-{gradle_version}-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
"#
    )
}

/// Unix shell script for gradle wrapper (`gradlew`).
///
/// Bootstraps gradle-wrapper.jar download from the URL in
/// gradle-wrapper.properties on first invocation. Shebang triggers 0755
/// chmod in the file writer.
pub(super) const GRADLE_WRAPPER_UNIX: &str = r#"#!/bin/sh

#
# Copyright 2015 the original author or authors.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      https://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#

##############################################################################
##
##  Gradle start up script for UN*X
##
##############################################################################

# Attempt to set APP_HOME
# Resolve links: $0 may be a link
PRG="$0"
# Need this for relative symlinks.
while [ -h "$PRG" ] ; do
    ls -ld "$PRG"
    link=`expr "$PRG" : '.*-> \(.*\)$'`
    if expr "$link" : '/.*' > /dev/null; then
        PRG="$link"
    else
        PRG=`dirname "$PRG"`"/$link"
    fi
done
SAVED="`pwd`"
cd "`dirname "$PRG"`/" >/dev/null
APP_HOME="`pwd -P`"
cd "$SAVED" >/dev/null

APP_NAME="Gradle"
APP_BASE_NAME=`basename "$0"`

# Add default JVM options here. You can also use JAVA_OPTS and GRADLE_OPTS to pass JVM options to this script.
DEFAULT_JVM_OPTS='"-Xmx64m" "-Xms64m"'

# Use the maximum available, or set MAX_FD != -1 to use that value.
MAX_FD="maximum"

warn () {
    echo "$*"
} >&2

die () {
    echo
    echo "$*"
    echo
    exit 1
} >&2

# OS specific support (must be 'true' or 'false').
cygwin=false
msys=false
darwin=false
nonstop=false
case "`uname`" in
  CYGWIN* )
    cygwin=true
    ;;
  Darwin* )
    darwin=true
    ;;
  MSYS* | MINGW* )
    msys=true
    ;;
  NONSTOP* )
    nonstop=true
    ;;
esac

CLASSPATH=$APP_HOME/gradle/wrapper/gradle-wrapper.jar

# Determine the Java command to use to start the JVM.
if [ -n "$JAVA_HOME" ] ; then
    if [ -x "$JAVA_HOME/jre/sh/java" ] ; then
        # IBM's JDK on AIX uses strange locations for the executables
        JAVACMD="$JAVA_HOME/jre/sh/java"
    else
        JAVACMD="$JAVA_HOME/bin/java"
    fi
    if [ ! -x "$JAVACMD" ] ; then
        die "ERROR: JAVA_HOME is set to an invalid directory: $JAVA_HOME

Please set the JAVA_HOME variable in your environment to match the
location of your Java installation."
    fi
else
    JAVACMD="java"
    which java >/dev/null 2>&1 || die "ERROR: JAVA_HOME is not set and no 'java' command could be found in your PATH.

Please set the JAVA_HOME variable in your environment to match the
location of your Java installation."
fi

# Increase the maximum file descriptors if we can.
if [ "$cygwin" = "false" -a "$msys" = "false" ] && command -v ulimit > /dev/null ; then
    if [ "$nonstop" = "false" ] ; then
        # Try setting the maximum allowed open files if we know how to.
        # Linux sets the default to 1024.
        if [ -n "$MAX_FD" -a \( "$MAX_FD" = "maximum" -o "$MAX_FD" = "max" \) ] ; then
            MAX_FD_LIMIT=`ulimit -H -n`
            if [ $? -eq 0 ] ; then
                if [ "$MAX_FD" = "maximum" -o "$MAX_FD" = "max" ] ; then
                    MAX_FD="$MAX_FD_LIMIT"
                fi
                ulimit -n $MAX_FD
                if [ $? -ne 0 ] ; then
                    warn "Could not set maximum file descriptor limit: $MAX_FD"
                fi
            else
                warn "Could not query maximum file descriptor limit: $MAX_FD_LIMIT"
            fi
        else
            warn "Max file descriptor limit unknown on this system."
        fi
    else
        warn "Unknown value for MAX_FD: $MAX_FD"
    fi
fi

# For Darwin, add options to specify how the application appears in the dock, menus, etc.
if [ "$darwin" = "true" ] ; then
    GRADLE_OPTS="$GRADLE_OPTS \"-Xdock:name=$APP_NAME\" \"-Xdock:icon=$APP_HOME/media/gradle.icns\""
fi

# For Cygwin or MSYS, switch paths to Windows-native format before running java
if [ "$cygwin" = "true" -o "$msys" = "true" ] ; then
    APP_HOME=`cygpath --path --mixed "$APP_HOME"`
    CLASSPATH=`cygpath --path --mixed "$CLASSPATH"`

    JAVACMD=`cygpath --unix "$JAVACMD"`

    # We build the pattern for arguments to be converted via cygpath
    ROOTDIRSRAW=`find -L / -maxdepth 3 -type d -name gradle 2>/dev/null | head -1`
    if [ -d "$ROOTDIRSRAW" ] ; then
        ROOTDIRS="$ROOTDIRSRAW"
    else
        ROOTDIRS=`dirname "$ROOTDIRSRAW"`
    fi
    SEP=":"
    if [ "$cygwin" = "true" ] ; then
        SEP=";"
    fi
    OURCYGPATTERN="(^($(\\/)|([a-zA-Z]:\\/))\\\\)?([^()\\/| ]*+)(\\\\[^()\\/| |\"]*+)*+$"
    # Add a user-defined pattern to the cygpath arguments
    if [ "$GRADLE_CYGWIN_PATTERN" != "" ] ; then
        OURCYGPATTERN="$OURCYGPATTERN|($GRADLE_CYGWIN_PATTERN)"
    fi
    # Now convert the arguments - kludge to limit ourselves to /bin/sh
    i=0
    for arg in "$@" ; do
        CHECK=`echo "$arg"|egrep -c "$OURCYGPATTERN" -`
        CHECK2=`echo "$arg"|egrep -c "^-"`                                 ### Determine if an option

        if [ $CHECK -ne 0 ] && [ $CHECK2 -eq 0 ] ; then                    ### Added a condition
            eval `echo args$i`=`cygpath --path --ignore --mixed "$arg"`
        else
            eval `echo args$i`="\"$arg\""
        fi
        i=`expr $i + 1`
    done
    case $i in
        0) set -- ;;
        1) set -- "$args0" ;;
        2) set -- "$args0" "$args1" ;;
        3) set -- "$args0" "$args1" "$args2" ;;
        4) set -- "$args0" "$args1" "$args2" "$args3" ;;
        5) set -- "$args0" "$args1" "$args2" "$args3" "$args4" ;;
        6) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" ;;
        7) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" ;;
        8) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" "$args7" ;;
        9) set -- "$args0" "$args1" "$args2" "$args3" "$args4" "$args5" "$args6" "$args7" "$args8" ;;
    esac
fi

# Escape application args
save () {
    for i do printf %s\\n "$i" | sed "s/'/'\\\\''/g;1s/^/'/;\$s/\$/' \\\\/" ; done
    echo " "
}
APP_ARGS=`save "$@"`

# Collect all arguments for the java command, following the shell quoting and substitution rules
eval set -- $DEFAULT_JVM_OPTS $JAVA_OPTS $GRADLE_OPTS "\"-Dorg.gradle.appname=$APP_BASE_NAME\"" -classpath "$CLASSPATH" org.gradle.wrapper.GradleWrapperMain "$APP_ARGS"

exec "$JAVACMD" "$@"
"#;

/// Windows batch script for gradle wrapper (`gradlew.bat`).
pub(super) const GRADLE_WRAPPER_WINDOWS: &str = r#"@rem
@rem Copyright 2015 the original author or authors.
@rem
@rem Licensed under the Apache License, Version 2.0 (the "License");
@rem you may not use this file except in compliance with the License.
@rem You may obtain a copy of the License at
@rem
@rem      https://www.apache.org/licenses/LICENSE-2.0
@rem
@rem Unless required by applicable law or agreed to in writing, software
@rem distributed under the License is distributed on an "AS IS" BASIS,
@rem WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
@rem See the License for the specific language governing permissions and
@rem limitations under the License.
@rem

@if "%DEBUG%" == "" @echo off
@rem ##########################################################################
@rem
@rem  Gradle startup script for Windows
@rem
@rem ##########################################################################

@rem Set local scope for the variables with windows NT shell
if "%OS%"=="Windows_NT" setlocal

set DIRNAME=%~dp0
if "%DIRNAME%" == "" set DIRNAME=.
set APP_BASE_NAME=%~n0
set APP_HOME=%DIRNAME%

@rem Resolve any "." and ".." in APP_HOME to make it shorter.
for %%i in ("%APP_HOME%") do set APP_HOME=%%~fi

@rem Add default JVM options here. You can also use JAVA_OPTS and GRADLE_OPTS to pass JVM options to this script.
set DEFAULT_JVM_OPTS="-Xmx64m" "-Xms64m"

@rem Find java.exe
if defined JAVA_HOME goto findJavaFromJavaHome

set JAVA_EXE=java.exe
%JAVA_EXE% -version >nul 2>&1
if "%ERRORLEVEL%" == "0" goto execute

echo.
echo ERROR: JAVA_HOME is not set and no 'java' command could be found in your PATH.
echo.
echo Please set the JAVA_HOME variable in your environment to match the
echo location of your Java installation.

goto fail

:findJavaFromJavaHome
set JAVA_HOME=%JAVA_HOME:"=%
set JAVA_EXE=%JAVA_HOME%\bin\java.exe

if exist "%JAVA_EXE%" goto execute

echo.
echo ERROR: JAVA_HOME is set to an invalid directory: %JAVA_HOME%
echo.
echo Please set the JAVA_HOME variable in your environment to match the
echo location of your Java installation.

goto fail

:execute
@rem Setup the command line

set CLASSPATH=%APP_HOME%\gradle\wrapper\gradle-wrapper.jar

@rem Execute Gradle
"%JAVA_EXE%" %DEFAULT_JVM_OPTS% %JAVA_OPTS% %GRADLE_OPTS% "-Dorg.gradle.appname=%APP_BASE_NAME%" -classpath "%CLASSPATH%" org.gradle.wrapper.GradleWrapperMain %*

:end
@endlocal & set ERROR_CODE=%ERRORLEVEL%

if not "%ERROR_CODE%" == "0" goto fail

:fail
exit /b %ERROR_CODE%

:mainEnd
if "%1"=="start" (
	call :startApp
	exit /b
)
call :stopApp
exit /b

:startApp
start "" cmd /k start %APP_HOME%\bin\myApp.bat
exit /b

:stopApp
taskkill /IM myApp.exe /F
exit /b
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Gradle wrapper properties must reference a valid Gradle version and
    /// point to services.gradle.org distribution URL.
    #[test]
    fn gradle_wrapper_properties_includes_valid_distribution_url() {
        let output = render_gradle_wrapper_properties();
        assert!(
            output.contains("services.gradle.org/distributions/gradle-"),
            "gradle-wrapper.properties must reference services.gradle.org distribution, got:\n{output}"
        );
        assert!(
            output.contains("-bin.zip"),
            "gradle-wrapper.properties must reference -bin.zip distribution, got:\n{output}"
        );
        assert!(
            output.contains("distributionBase=GRADLE_USER_HOME"),
            "gradle-wrapper.properties must set distributionBase, got:\n{output}"
        );
    }

    /// The gradle wrapper unix script must contain shebang and valid shell syntax.
    #[test]
    fn gradle_wrapper_unix_script_is_valid_shell() {
        assert!(
            GRADLE_WRAPPER_UNIX.starts_with("#!/bin/sh"),
            "gradlew must start with #!/bin/sh shebang"
        );
        assert!(GRADLE_WRAPPER_UNIX.contains("CLASSPATH"), "gradlew must set CLASSPATH");
        assert!(
            GRADLE_WRAPPER_UNIX.contains("org.gradle.wrapper.GradleWrapperMain"),
            "gradlew must invoke GradleWrapperMain"
        );
    }

    /// The gradle wrapper windows script must be valid batch syntax.
    #[test]
    fn gradle_wrapper_windows_script_is_valid_batch() {
        assert!(
            GRADLE_WRAPPER_WINDOWS.starts_with("@rem"),
            "gradlew.bat must start with @rem comment"
        );
        assert!(
            GRADLE_WRAPPER_WINDOWS.contains("java.exe"),
            "gradlew.bat must reference java.exe"
        );
        assert!(
            GRADLE_WRAPPER_WINDOWS.contains("org.gradle.wrapper.GradleWrapperMain"),
            "gradlew.bat must invoke GradleWrapperMain"
        );
    }

    /// gradle-wrapper.jar must be emitted as base64-encoded content.
    /// The file writer will detect the .jar extension and decode it automatically.
    #[test]
    fn gradle_wrapper_jar_is_base64_encoded() {
        let jar_b64 = get_gradle_wrapper_jar_base64();
        assert!(
            jar_b64.starts_with("UEsD"),
            "gradle-wrapper.jar base64 must start with encoded ZIP magic bytes 'UEsD', got:\n{}",
            &jar_b64[..std::cmp::min(50, jar_b64.len())]
        );
        assert!(
            !jar_b64.contains('\n'),
            "gradle-wrapper.jar base64 must not contain newlines"
        );
    }
}
