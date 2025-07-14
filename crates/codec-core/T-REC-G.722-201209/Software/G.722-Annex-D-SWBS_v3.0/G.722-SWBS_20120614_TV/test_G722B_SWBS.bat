echo off

set OUT_DIR=out
set TEST_VEC=tv
set TRUNCATE=.\tools\truncate
set EID=.\tools\eid-xor

del *.pk
del *.txt
del *.bak

if %1 EQU test (
	call :proc
	call :comp
	pause
)
if %1 EQU proc (
 	call :proc
 )
 if %1 EQU comp (
 	call :comp
 )

goto :end

:proc

	if not exist %OUT_DIR%\ (
		mkdir %OUT_DIR%
	)

del *.pk
del *.bak
del *.txt
del *_tv.bit
del *_tv.raw

rem WB STEREO 64kbps
g722_stereo_enc -wb -stereo %TEST_VEC%\signal_st_16kHz.raw %OUT_DIR%\signal_st_16kHz_64kbps.bit 64
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_64kbps.bit %OUT_DIR%\signal_st_16kHz_64kbps.raw 64
rem WB STEREO 64kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_64_56 %OUT_DIR%\signal_st_16kHz_64kbps.bit %OUT_DIR%\signal_st_16kHz_64_56kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_64_56kbps.bit %OUT_DIR%\signal_st_16kHz_64_56kbps.raw 64 -bitrateswitch 1
rem WB STEREO 64kbps FEC
%EID% -fer %OUT_DIR%\signal_st_16kHz_64kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_16kHz_64kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_64kbps_FER3.bit %OUT_DIR%\signal_st_16kHz_64kbps_FER3.raw 64

rem WB STEREO 80kbps
g722_stereo_enc -wb -stereo %TEST_VEC%\signal_st_16kHz.raw %OUT_DIR%\signal_st_16kHz_80kbps.bit 80
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_80kbps.bit %OUT_DIR%\signal_st_16kHz_80kbps.raw 80
rem WB STEREO 80kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_80_64 %OUT_DIR%\signal_st_16kHz_80kbps.bit %OUT_DIR%\signal_st_16kHz_80_64kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_80_64kbps.bit %OUT_DIR%\signal_st_16kHz_80_64kbps.raw 80 -bitrateswitch 0
rem WB STEREO 80kbps FEC
%EID% -fer %OUT_DIR%\signal_st_16kHz_80kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_16kHz_80kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_16kHz_80kbps_FER3.bit %OUT_DIR%\signal_st_16kHz_80kbps_FER3.raw 80

rem SWB STEREO 80kbps
g722_stereo_enc -stereo %TEST_VEC%\signal_st_32kHz.raw %OUT_DIR%\signal_st_32kHz_80kbps.bit 80
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_80kbps.bit %OUT_DIR%\signal_st_32kHz_80kbps.raw 80
rem SWB STEREO 80kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_80_64_56 %OUT_DIR%\signal_st_32kHz_80kbps.bit %OUT_DIR%\signal_st_32kHz_80_64_56kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_80_64_56kbps.bit %OUT_DIR%\signal_st_32kHz_80_64_56kbps.raw 80 -bitrateswitch 1
rem SWB STEREO 80kbps FEC
%EID% -fer %OUT_DIR%\signal_st_32kHz_80kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_32kHz_80kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_80kbps_FER3.bit %OUT_DIR%\signal_st_32kHz_80kbps_FER3.raw 80

rem SWB STEREO 96kbps
g722_stereo_enc -stereo %TEST_VEC%\signal_st_32kHz.raw %OUT_DIR%\signal_st_32kHz_96kbps.bit 96
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_96kbps.bit %OUT_DIR%\signal_st_32kHz_96kbps.raw 96
rem SWB STEREO 96kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_96_80_64 %OUT_DIR%\signal_st_32kHz_96kbps.bit %OUT_DIR%\signal_st_32kHz_96_80_64kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_96_80_64kbps.bit %OUT_DIR%\signal_st_32kHz_96_80_64kbps.raw 96 -bitrateswitch 0
rem SWB STEREO 96kbps FEC
%EID% -fer %OUT_DIR%\signal_st_32kHz_96kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_32kHz_96kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_96kbps_FER3.bit %OUT_DIR%\signal_st_32kHz_96kbps_FER3.raw 96

rem SWB STEREO 112kbps
g722_stereo_enc -stereo %TEST_VEC%\signal_st_32kHz.raw %OUT_DIR%\signal_st_32kHz_112kbps.bit 112
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_112kbps.bit %OUT_DIR%\signal_st_32kHz_112kbps.raw 112
rem SWB STEREO 112kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_112_96_80_64 %OUT_DIR%\signal_st_32kHz_112kbps.bit %OUT_DIR%\signal_st_32kHz_112_96_80_64kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_112_96_80_64kbps.bit %OUT_DIR%\signal_st_32kHz_112_96_80_64kbps.raw 112 -bitrateswitch 0
rem SWB STEREO 112kbps FEC
%EID% -fer %OUT_DIR%\signal_st_32kHz_112kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_32kHz_112kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_112kbps_FER3.bit %OUT_DIR%\signal_st_32kHz_112kbps_FER3.raw 112

rem SWB STEREO 128kbps
g722_stereo_enc -stereo %TEST_VEC%\signal_st_32kHz.raw %OUT_DIR%\signal_st_32kHz_128kbps.bit 128
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_128kbps.bit %OUT_DIR%\signal_st_32kHz_128kbps.raw 128
rem SWB STEREO 128kbps rate switching
%TRUNCATE% -q -fl 5 -bf %TEST_VEC%\g722st_btrF_128_112_96_80_64 %OUT_DIR%\signal_st_32kHz_128kbps.bit %OUT_DIR%\signal_st_32kHz_128_112_96_80_64kbps.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_128_112_96_80_64kbps.bit %OUT_DIR%\signal_st_32kHz_128_112_96_80_64kbps.raw 128 -bitrateswitch 0
rem SWB STEREO 128kbps FEC
%EID% -fer %OUT_DIR%\signal_st_32kHz_128kbps.bit %TEST_VEC%\FER3.g192 %OUT_DIR%\signal_st_32kHz_128kbps_FER3.bit
g722_stereo_dec -stereo %OUT_DIR%\signal_st_32kHz_128kbps_FER3.bit %OUT_DIR%\signal_st_32kHz_128kbps_FER3.raw 128

goto :eof

:comp

REM BE check
rem WB STEREO 64kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_64kbps.bit %TEST_VEC%\signal_st_16kHz_64kbps_tv.bit >TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_64kbps.raw %TEST_VEC%\signal_st_16kHz_64kbps_tv.raw >>TV_check_results.txt
rem WB STEREO 64kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_64_56kbps.raw %TEST_VEC%\signal_st_16kHz_64_56kbps_tv.raw >>TV_check_results.txt
rem WB STEREO 64kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_64kbps_FER3.raw %TEST_VEC%\signal_st_16kHz_64kbps_FER3_tv.raw >>TV_check_results.txt

rem WB STEREO 80kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_80kbps.bit %TEST_VEC%\signal_st_16kHz_80kbps_tv.bit >>TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_80kbps.raw %TEST_VEC%\signal_st_16kHz_80kbps_tv.raw >>TV_check_results.txt
rem WB STEREO 80kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_80_64kbps.raw %TEST_VEC%\signal_st_16kHz_80_64kbps_tv.raw >>TV_check_results.txt
rem WB STEREO 80kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_16kHz_80kbps_FER3.raw %TEST_VEC%\signal_st_16kHz_80kbps_FER3_tv.raw >>TV_check_results.txt

rem SWB STEREO 80kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_80kbps.bit %TEST_VEC%\signal_st_32kHz_80kbps_tv.bit >>TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_80kbps.raw %TEST_VEC%\signal_st_32kHz_80kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 80kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_80_64_56kbps.raw %TEST_VEC%\signal_st_32kHz_80_64_56kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 80kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_80kbps_FER3.raw %TEST_VEC%\signal_st_32kHz_80kbps_FER3_tv.raw >>TV_check_results.txt

rem SWB STEREO 96kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_96kbps.bit %TEST_VEC%\signal_st_32kHz_96kbps_tv.bit >>TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_96kbps.raw %TEST_VEC%\signal_st_32kHz_96kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 96kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_96_80_64kbps.raw %TEST_VEC%\signal_st_32kHz_96_80_64kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 96kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_96kbps_FER3.raw %TEST_VEC%\signal_st_32kHz_96kbps_FER3_tv.raw >>TV_check_results.txt

rem SWB STEREO 112kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_112kbps.bit %TEST_VEC%\signal_st_32kHz_112kbps_tv.bit >>TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_112kbps.raw %TEST_VEC%\signal_st_32kHz_112kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 112kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_112_96_80_64kbps.raw %TEST_VEC%\signal_st_32kHz_112_96_80_64kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 112kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_112kbps_FER3.raw %TEST_VEC%\signal_st_32kHz_112kbps_FER3_tv.raw >>TV_check_results.txt

rem SWB STEREO 128kbps
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_128kbps.bit %TEST_VEC%\signal_st_32kHz_128kbps_tv.bit >>TV_check_results.txt
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_128kbps.raw %TEST_VEC%\signal_st_32kHz_128kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 128kbps rate switching
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_128_112_96_80_64kbps.raw %TEST_VEC%\signal_st_32kHz_128_112_96_80_64kbps_tv.raw >>TV_check_results.txt
rem SWB STEREO 128kbps FEC
tools\BitExactness_checking  %OUT_DIR%\signal_st_32kHz_128kbps_FER3.raw %TEST_VEC%\signal_st_32kHz_128kbps_FER3_tv.raw >>TV_check_results.txt

goto :eof
:end

echo  ------------------------
echo    END OF Processing
echo  ------------------------


pause