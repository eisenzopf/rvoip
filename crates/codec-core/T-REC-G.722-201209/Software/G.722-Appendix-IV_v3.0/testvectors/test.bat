@echo off
REM  Usage :
REM   - put binary decoder g722dec in "testvectors" folder.
REM   - run test.bat
REM   - check that no differences are found between the reference files and the processed files *.p*

del /Q *.pout

decg722.exe -fsize 160 .\TV\test10.bst test10.pout
decg722.exe -fsize 320 .\TV\test20.bst test20.pout
decg722.exe -fsize 320 .\TV\ovfl.bst ovfl.pout

FC /B .\TV\test10.out test10.pout
FC /B .\TV\test20.out test20.pout
FC /B .\TV\ovfl.out ovfl.pout

