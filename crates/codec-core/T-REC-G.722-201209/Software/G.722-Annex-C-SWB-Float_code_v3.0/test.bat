echo off

set TEST_DIR=src\test_codec
set TEST_VEC=test_vector_mswin
set OUT_DIR=out
set DIFF=fc /b /a
set TRUNCATE=.\tools\truncate
set EID=.\tools\eid-xor
set FILTER=.\tools\filter

if %1 EQU test (
	call :proc
	call :comp
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

rem
rem  SuperWideband Input
rem

rem G.722 core at 56 kbit/s
	%TEST_DIR%\encoder %TEST_VEC%\Signal.inp tmp_R1sm.bit 64
	%TEST_DIR%\encoder %TEST_VEC%\Signal.inp tmp_R3sm.bit 96
	
	%TRUNCATE% -fl 5 -b 64000 tmp_R1sm.bit %OUT_DIR%\Signal_R1sm.bit
	%TRUNCATE% -fl 5 -b 80000 tmp_R3sm.bit %OUT_DIR%\Signal_R2sm.bit
	%TRUNCATE% -fl 5 -b 96000 tmp_R3sm.bit %OUT_DIR%\Signal_R3sm.bit
	
	%EID% -fer %OUT_DIR%\Signal_R1sm.bit %TEST_VEC%\FER_pattern.g192 %OUT_DIR%\Signal_R1sm_FER3.bit
	%EID% -fer %OUT_DIR%\Signal_R2sm.bit %TEST_VEC%\FER_pattern.g192 %OUT_DIR%\Signal_R2sm_FER3.bit
	%EID% -fer %OUT_DIR%\Signal_R3sm.bit %TEST_VEC%\FER_pattern.g192 %OUT_DIR%\Signal_R3sm_FER3.bit
	
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R1sm.bit %OUT_DIR%\Signal_R1sm.out 64
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R2sm.bit %OUT_DIR%\Signal_R2sm.out 80
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R3sm.bit %OUT_DIR%\Signal_R3sm.out 96
	
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R1sm_FER3.bit %OUT_DIR%\Signal_R1sm_FER3.out 64
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R2sm_FER3.bit %OUT_DIR%\Signal_R2sm_FER3.out 80
	%TEST_DIR%\decoder %OUT_DIR%\Signal_R3sm_FER3.bit %OUT_DIR%\Signal_R3sm_FER3.out 96

	del tmp_R1sm.bit tmp_R3sm.bit
	
goto :eof

:comp
	
rem
rem  SuperWideband Input
rem
	%DIFF% %OUT_DIR%\Signal_R1sm.bit %OUT_DIR%\Signal_R1sm.bit
	%DIFF% %OUT_DIR%\Signal_R2sm.bit %OUT_DIR%\Signal_R2sm.bit
	%DIFF% %OUT_DIR%\Signal_R3sm.bit %OUT_DIR%\Signal_R3sm.bit
	%DIFF% %OUT_DIR%\Signal_R1sm_FER3.bit %OUT_DIR%\Signal_R1sm_FER3.bit
	%DIFF% %OUT_DIR%\Signal_R2sm_FER3.bit %OUT_DIR%\Signal_R2sm_FER3.bit
	%DIFF% %OUT_DIR%\Signal_R3sm_FER3.bit %OUT_DIR%\Signal_R3sm_FER3.bit

	%DIFF% %OUT_DIR%\Signal_R1sm.out %OUT_DIR%\Signal_R1sm.out
	%DIFF% %OUT_DIR%\Signal_R2sm.out %OUT_DIR%\Signal_R2sm.out
	%DIFF% %OUT_DIR%\Signal_R3sm.out %OUT_DIR%\Signal_R3sm.out
	%DIFF% %OUT_DIR%\Signal_R1sm_FER3.out %OUT_DIR%\Signal_R1sm_FER3.out
	%DIFF% %OUT_DIR%\Signal_R2sm_FER3.out %OUT_DIR%\Signal_R2sm_FER3.out
	%DIFF% %OUT_DIR%\Signal_R3sm_FER3.out %OUT_DIR%\Signal_R3sm_FER3.out

goto :eof
:end

echo  ------------------------
echo    END OF Processing
echo  ------------------------
