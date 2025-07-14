/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G.722 Appendix III                                                      */
/* Version:       1.0                                                            */
/* Revision Date: Nov.02, 2006                                                   */

/*
  ITU-T G.722 Appendix III ANSI-C Source Code
  Copyright (c)  Broadcom Corporation 2006.
*/

#include <stdlib.h>
#include <stdio.h>
#include "typedef.h"
#include "memutil.h"
#include "g722plc.h"


extern MEM_OP dmemCounter;

/* allocate a Word16 vector with subscript range v[nl...nh] */
/* from numerical recipes in C 2nd edition, page 943        */
Word16 *allocWord16(long nl, long nh)
{
  Word16 *v;

#if (DMEM)
  dmemCounter.allocWord16 += (nh-nl+1);
  Test_DMEM_Counter();
#endif

  v = (Word16 *)malloc((size_t)((nh-nl+1)*sizeof(Word16)));
  if (!v){
    printf("Memory allocation error in allocWord16()\n");
    exit(0);
  }

  return v-nl;
}

/* free a Word16 vector allocated by svector()              */
/* from numerical recipes in C 2nd edition, page 946        */
void deallocWord16(Word16 *v, long nl, long nh)
{

#if (DMEM)
  dmemCounter.allocWord16 -= (nh-nl+1);
#endif

  free((char *)(v+nl));

  return;
}

/* allocate a Word32 vector with subscript range v[nl...nh] */
/* from numerical recipes in C 2nd edition, page 943        */
Word32 *allocWord32(long nl, long nh)
{
  Word32 *v;

#if (DMEM)
  dmemCounter.allocWord32 += (nh-nl+1);
  Test_DMEM_Counter();
#endif

  v = (Word32 *)malloc((size_t)((nh-nl+1)*sizeof(Word32)));
  if (!v){
    printf("Memory allocation error in allocWord32()\n");
    exit(0);
  }

  return v-nl;
}

/* free a Word32 vector allocated by svector()              */
/* from numerical recipes in C 2nd edition, page 946        */
void deallocWord32(Word32 *v, long nl, long nh)
{

#if (DMEM)
  dmemCounter.allocWord32 -= (nh-nl+1);
#endif

  free((char *)(v+nl));

  return;
}

/* Global counter variable for calculation of dynamic memory usage */
MEM_OP dmemCounter;

/* Initialize dynamic memory counter */
void Init_DMEM_Counter(){
  
  dmemCounter.allocWord16 = 0;
  dmemCounter.allocWord32 = 0;
  dmemCounter.WorstCase = 0;

  return;
}

/* Test dynamic memory usage */
void Test_DMEM_Counter(){
  Word32 new;

  new = 
    dmemCounter.allocWord16 * 2 + /* 2 bytes per element */
    dmemCounter.allocWord32 * 4;  /* 4 bytes per element */

  if(new > dmemCounter.WorstCase)
    dmemCounter.WorstCase = new;

  return;
}

/* Test for dynamic memory leakage */
void Test_DMEM_Leakage(){
  
  if(dmemCounter.allocWord16 != 0)
    fprintf(stderr, "\nWARNING: Memory leakage (Word16)!!!\n");
  
  if(dmemCounter.allocWord32 != 0)
    fprintf(stderr, "\nWARNING: Memory leakage (Word32)!!!\n");

  return;
}

/* Output dynamic memory usage */
void DMEM_output(){

  fprintf (stdout, "   Worst Case PLC Scratch Mem Usage: %d bytes\n",dmemCounter.WorstCase);
  fprintf (stdout, "   PLC State Mem Usage             : %d bytes\n",sizeof(struct WB_PLC_State));
  fprintf (stdout, "   Total Mem Usage                 : %d bytes\n",sizeof(struct WB_PLC_State)+dmemCounter.WorstCase);
  return;
}
