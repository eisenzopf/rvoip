/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

Word16 *allocWord16(long nl, long nh);
void deallocWord16(Word16 *v, long nl, long nh);
Word32 *allocWord32(long nl, long nh);
void deallocWord32(Word32 *v, long nl, long nh);
void Init_DMEM_Counter();
void Test_DMEM_Counter();
void Test_DMEM_Leakage();
void DMEM_output();

typedef struct
{
  Word32 allocWord16;
  Word32 allocWord32;
  Word32 WorstCase;
}
MEM_OP;
