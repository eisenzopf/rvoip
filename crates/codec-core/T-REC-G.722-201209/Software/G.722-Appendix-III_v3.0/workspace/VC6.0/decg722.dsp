# Microsoft Developer Studio Project File - Name="decg722" - Package Owner=<4>
# Microsoft Developer Studio Generated Build File, Format Version 6.00
# ** DO NOT EDIT **

# TARGTYPE "Win32 (x86) Console Application" 0x0103

CFG=decg722 - Win32 Debug
!MESSAGE This is not a valid makefile. To build this project using NMAKE,
!MESSAGE use the Export Makefile command and run
!MESSAGE 
!MESSAGE NMAKE /f "decg722.mak".
!MESSAGE 
!MESSAGE You can specify a configuration when running NMAKE
!MESSAGE by defining the macro CFG on the command line. For example:
!MESSAGE 
!MESSAGE NMAKE /f "decg722.mak" CFG="decg722 - Win32 Debug"
!MESSAGE 
!MESSAGE Possible choices for configuration are:
!MESSAGE 
!MESSAGE "decg722 - Win32 Release" (based on "Win32 (x86) Console Application")
!MESSAGE "decg722 - Win32 Debug" (based on "Win32 (x86) Console Application")
!MESSAGE 

# Begin Project
# PROP AllowPerConfigDependencies 0
# PROP Scc_ProjName ""
# PROP Scc_LocalPath ""
CPP=cl.exe
RSC=rc.exe

!IF  "$(CFG)" == "decg722 - Win32 Release"

# PROP BASE Use_MFC 0
# PROP BASE Use_Debug_Libraries 0
# PROP BASE Output_Dir "Release"
# PROP BASE Intermediate_Dir "Release"
# PROP BASE Target_Dir ""
# PROP Use_MFC 0
# PROP Use_Debug_Libraries 0
# PROP Output_Dir "..\..\Release"
# PROP Intermediate_Dir "..\..\Release"
# PROP Target_Dir ""
# ADD BASE CPP /nologo /W3 /GX /O2 /D "WIN32" /D "NDEBUG" /D "_CONSOLE" /D "_MBCS" /YX /FD /c
# ADD CPP /nologo /W3 /GX /Od /I "..\..\src\stl2005_basop" /I "..\..\src\g722plc" /I "..\..\src" /D "WIN32" /D "NDEBUG" /D "_CONSOLE" /D "_MBCS" /D DMEM=1 /D WMOPS=1 /Fr /YX /FD /c
# ADD BASE RSC /l 0x40c /d "NDEBUG"
# ADD RSC /l 0x40c /d "NDEBUG"
BSC32=bscmake.exe
# ADD BASE BSC32 /nologo
# ADD BSC32 /nologo
LINK32=link.exe
# ADD BASE LINK32 kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib /nologo /subsystem:console /machine:I386
# ADD LINK32 kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib /nologo /subsystem:console /machine:I386

!ELSEIF  "$(CFG)" == "decg722 - Win32 Debug"

# PROP BASE Use_MFC 0
# PROP BASE Use_Debug_Libraries 1
# PROP BASE Output_Dir "decg722___Win32_Debug"
# PROP BASE Intermediate_Dir "decg722___Win32_Debug"
# PROP BASE Target_Dir ""
# PROP Use_MFC 0
# PROP Use_Debug_Libraries 1
# PROP Output_Dir "..\..\Debug"
# PROP Intermediate_Dir "..\..\Debug"
# PROP Ignore_Export_Lib 0
# PROP Target_Dir ""
# ADD BASE CPP /nologo /W3 /Gm /GX /ZI /Od /D "WIN32" /D "_DEBUG" /D "_CONSOLE" /D "_MBCS" /YX /FD /GZ /c
# ADD CPP /nologo /W3 /Gm /GX /ZI /Od /I "..\..\src\stl2005_basop" /I "..\..\src\g722plc" /I "..\..\src" /D "WIN32" /D "_DEBUG" /D "_CONSOLE" /D "_MBCS" /D DMEM=1 /D WMOPS=1 /FR /YX /FD /GZ /c
# ADD BASE RSC /l 0x40c /d "_DEBUG"
# ADD RSC /l 0x40c /d "_DEBUG"
BSC32=bscmake.exe
# ADD BASE BSC32 /nologo
# ADD BSC32 /nologo
LINK32=link.exe
# ADD BASE LINK32 kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib /nologo /subsystem:console /debug /machine:I386 /pdbtype:sept
# ADD LINK32 kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib kernel32.lib user32.lib gdi32.lib winspool.lib comdlg32.lib advapi32.lib shell32.lib ole32.lib oleaut32.lib uuid.lib odbc32.lib odbccp32.lib /nologo /subsystem:console /debug /machine:I386 /pdbtype:sept

!ENDIF 

# Begin Target

# Name "decg722 - Win32 Release"
# Name "decg722 - Win32 Debug"
# Begin Group "Source Files"

# PROP Default_Filter "cpp;c;cxx;rc;def;r;odl;idl;hpj;bat"
# Begin Group "stl2005"

# PROP Default_Filter ""
# Begin Source File

SOURCE=..\..\src\stl2005_basop\basop32.c
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\control.c
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\count.c
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\enh1632.c
# End Source File
# End Group
# Begin Group "plc"

# PROP Default_Filter ""
# Begin Source File

SOURCE=..\..\src\g722plc\apfilter.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\autocor.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\azfilter.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\coarptch.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\decim.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\dspfunc.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\g722plc.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\levinson.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\memutil.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\merit.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\ppchange.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\prfn.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\table.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\utility.c
# End Source File
# End Group
# Begin Source File

SOURCE=..\..\src\decg722.c
# End Source File
# Begin Source File

SOURCE=..\..\src\funcg722.c
# End Source File
# Begin Source File

SOURCE=..\..\src\g722.c
# End Source File
# Begin Source File

SOURCE=..\..\src\softbit.c
# End Source File
# End Group
# Begin Group "Header Files"

# PROP Default_Filter "h;hpp;hxx;hm;inl"
# Begin Group "stl2005_h"

# PROP Default_Filter ""
# Begin Source File

SOURCE=..\..\src\stl2005_basop\basop32.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\control.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\count.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\enh1632.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\move.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\patch.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\stl.h
# End Source File
# Begin Source File

SOURCE=..\..\src\stl2005_basop\typedef.h
# End Source File
# End Group
# Begin Group "plc_h"

# PROP Default_Filter ""
# Begin Source File

SOURCE=..\..\src\g722plc\g722plc.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\memutil.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\ppchange.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\table.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722plc\utility.h
# End Source File
# End Group
# Begin Source File

SOURCE=..\..\src\funcg722.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722.h
# End Source File
# Begin Source File

SOURCE=..\..\src\g722_com.h
# End Source File
# Begin Source File

SOURCE=..\..\src\softbit.h
# End Source File
# Begin Source File

SOURCE=..\..\src\ugstdemo.h
# End Source File
# End Group
# End Target
# End Project
