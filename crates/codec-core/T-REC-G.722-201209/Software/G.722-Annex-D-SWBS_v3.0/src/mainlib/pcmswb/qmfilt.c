/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

/*
*------------------------------------------------------------------------
*  File: qmfilt.c
*  Function: Quadrature mirror filter (QMF)
*            for band splitting and band reconstructing
*------------------------------------------------------------------------
*/

#include "pcmswb_common.h"
#include "qmfilt.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

#define LFRAME_HALF        (L_FRAME_SWB/2)
#define NTAP_QMF_HALF_MAX  (NTAP_QMF_WB/2)
#define QMFBUFSIZE_MAX     (NTAP_QMF_WB-2)

typedef struct {
  Word16  ntap;
  Word16  *bufmem;
  Word16  ovflag_pre;
  const Word16 *sQmf0;
  const Word16 *sQmf1;
} QMFilt_WORK;

/* Constructor */
void* QMFilt_const(Word16 ntap, const Word16 *qmf0, const Word16 *qmf1)  /* returns pointer to work space */
{
  QMFilt_WORK *work=NULL;
  Word16 qmfbufsize = sub( ntap, 2);


  work = (QMFilt_WORK *)malloc(sizeof(QMFilt_WORK));
  if (work != NULL) {
    /*****************************/
#ifdef DYN_RAM_CNT
    {
      UWord32 ssize;
      ssize = (UWord32) (0);
#ifdef MEM_STT
      ssize += (UWord32) (sizeof(QMFilt_WORK));
#endif
      DYN_RAM_PUSH(ssize, "dummy");
    }
#endif
    /*****************************/
    work->ntap = ntap; move16();
    work->sQmf0 = qmf0; move16();
    work->sQmf1 = qmf1; move16();
    work->bufmem = (Word16 *)malloc(qmfbufsize * sizeof(Word16));
    if (work->bufmem != NULL) {
      /*****************************/
#ifdef DYN_RAM_CNT
      {
        UWord32 ssize;
        ssize = (UWord32) (0);
#ifdef MEM_STT
        ssize += (UWord32) (qmfbufsize * sizeof(Word16));
#endif
        DYN_RAM_PUSH(ssize, "dummy");
      }
#endif
      /*****************************/
      QMFilt_reset((void *)work);
    }
  }

  return (void *)work;
}

/* Destructor */
void  QMFilt_dest(void *ptr)
{
  QMFilt_WORK *work=(QMFilt_WORK *)ptr;

  if (work != NULL) {
    if (work->bufmem != NULL) {
      /*****************************/
#ifdef DYN_RAM_CNT
      DYN_RAM_POP();
#endif
      /*****************************/
      free(work->bufmem);
    }
    free(work);
    /*****************************/
#ifdef DYN_RAM_CNT
    DYN_RAM_POP();
#endif
    /*****************************/
  }
  return;
}

/* Reset */
void  QMFilt_reset(void *ptr)
{

  QMFilt_WORK *work=(QMFilt_WORK *)ptr;

#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) SIZE_Ptr, "dummy");
#endif

  if (work != NULL) {
    zero16(sub(work->ntap, 2), work->bufmem);
    work->ovflag_pre = 0; move16();
  }

#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
}

/* Band splitting */
void  QMFilt_ana(
                 Word16   n,      /* (i): Number of input signal      */
                 Word16  *insig,  /* (i): Input signal                */
                 Word16  *lsig,   /* (o): Output lower-band signal    */
                 Word16  *hsig,   /* (o): Output higher-band signal   */
                 void    *ptr     /* (i/o): Work space                */
                 ) 
{
  QMFilt_WORK *work=(QMFilt_WORK *)ptr;
  Word16     i, j;
  Word16     qmfbufsize = sub( work->ntap, 2);
  Word16     ntap_qmf_half = shr( work->ntap, 1);

  Word16  *insigpt;
  Word16  insigbf[L_FRAME_SWB+QMFBUFSIZE_MAX];
  Word32  lAcc0;
  Word32  lAcc1;
  const Word16 *sQmf0 = work->sQmf0;
  const Word16 *sQmf1 = work->sQmf1;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (4 * SIZE_Ptr + (L_FRAME_SWB + QMFBUFSIZE_MAX + 4) * SIZE_Word16 + 2 * SIZE_Word32), "dummy");
#endif
  /*****************************/
  insigpt = insigbf;

  mov16(qmfbufsize, work->bufmem, insigpt);
  insigpt = insigpt + qmfbufsize;
  mov16(n, insig, insigpt);
  insigpt = insigpt + n;

  FOR (i = 0; i < n; i+=2) {
    insigpt = insigbf + i;

    lAcc0 = L_mac(0, sQmf0[0], *(insigpt++));
    lAcc1 = L_mac(0, sQmf1[0], *(insigpt++));
    FOR (j = 1; j < ntap_qmf_half; j++) {
      lAcc0 = L_mac(lAcc0, sQmf0[j], *(insigpt++));
      lAcc1 = L_mac(lAcc1, sQmf1[j], *(insigpt++));
    }
    *(lsig++) = round_fx(L_add(lAcc0, lAcc1)); move16();
    *(hsig++) = round_fx(L_sub(lAcc1, lAcc0)); move16();
  }
  insigpt = insigbf + n;
  mov16(qmfbufsize, insigpt, work->bufmem);

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}

/* Band reconstructing */
void QMFilt_syn(
                Word16   n,      /* (i): Number of input signal       */
                Word16  *lsig,   /* (i): Input lower-band signal      */
                Word16  *hsig,   /* (i): Input higher-band signal     */
                Word16  *outsig, /* (o): Output 16-kHz-sampled signal */
                void    *ptr     /* (i/o): Pointer to work space      */
                ) 
{
  QMFilt_WORK *work=(QMFilt_WORK *)ptr;
  Word16     i, j;
  Word16     ntap_qmf_half = shr( work->ntap, 1);
  Word16  buf_sum[LFRAME_HALF+NTAP_QMF_HALF_MAX-1];
  Word16  buf_dif[LFRAME_HALF+NTAP_QMF_HALF_MAX-1];
  Word16  *pbuf;
  Word16  *ps_sum;
  Word16  *ps_dif;
  Word32  lAcc0;
  Word32  lAcc1;
  Word16  sAcc;
  Word16  ovflag;
  Word16  fmt, sshift;
  const Word16 *sQmf0 = work->sQmf0;
  const Word16 *sQmf1 = work->sQmf1;

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (6 * SIZE_Ptr + (2 * (LFRAME_HALF + NTAP_QMF_HALF_MAX - 1) + 7) * SIZE_Word16 + 2 * SIZE_Word32), "dummy");
#endif
  /*****************************/


  /* Overflow check */
  ovflag = 0; move16();
  FOR (i = 0; i < n; i++) {
    sAcc = sub (add(abs_s(lsig[i]), abs_s(hsig[i])), MAX_16);
    if (sAcc >= 0) {
      ovflag = 1; move16();
    }
  }

  pbuf = work->bufmem;
  ps_sum = buf_sum;  
  ps_dif = buf_dif;  

  /* copy from filter buffer */
  j = sub(work->ovflag_pre, ovflag);
  IF(j >= 0)
  {
    mov16_ext((Word16)(ntap_qmf_half-1), pbuf, 2, ps_sum, 1);
    mov16_ext((Word16)(ntap_qmf_half-1), pbuf+1, 2, ps_dif, 1);
  }
  ELSE
  {
    array_oper_ext((Word16)(ntap_qmf_half-1), 1, pbuf, 2, ps_sum, 1, &shr);
    array_oper_ext((Word16)(ntap_qmf_half-1), 1, pbuf+1, 2, ps_dif, 1, &shr);
  }

  fmt = s_max (ovflag, work->ovflag_pre);
  sshift = add (2, fmt);

  /* calculate sum/diff values */
  ps_sum = buf_sum+ntap_qmf_half-1;  
  ps_dif = buf_dif+ntap_qmf_half-1;  
  IF (fmt != 0)
  {
    FOR (i = 0; i < n; i++) {
      lAcc0 = L_mult (0x4000, lsig[i]);
      *(ps_sum++) = mac_r(lAcc0, 0x4000, hsig[i]);  /*Q(-1)*/ move16();
      *(ps_dif++) = msu_r(lAcc0, 0x4000, hsig[i]);  /*Q(-1)*/ move16();
    }
  }
  ELSE
  {
    FOR (i = 0; i < n; i++) {
      *(ps_sum++) = add(lsig[i], hsig[i]); move16();
      *(ps_dif++) = sub(lsig[i], hsig[i]); move16();
    }
  }

  ps_sum = buf_sum;
  ps_dif = buf_dif;

  FOR (i = 0; i < n; i++) {
    lAcc0 = L_mac0_Array(ntap_qmf_half, (Word16 *)sQmf0, ps_sum);
    lAcc1 = L_mac0_Array(ntap_qmf_half, (Word16 *)sQmf1, ps_dif);

    *(outsig++) = round_fx_L_shl(lAcc1, sshift); move16();
    *(outsig++) = round_fx_L_shl(lAcc0, sshift); move16();

    ps_sum++;
    ps_dif++;
  }

  /* copy to filter buffer */
  pbuf = work->bufmem;
  IF (ovflag != 0) {
    mov16_ext((Word16)(ntap_qmf_half-1), ps_sum, 1, pbuf, 2);
    mov16_ext((Word16)(ntap_qmf_half-1), ps_dif, 1, pbuf+1, 2);
  }
  ELSE IF (work->ovflag_pre == 0) { /*ovflag==0 && ovflag_pre==0*/
    mov16_ext((Word16)(ntap_qmf_half-1), ps_sum, 1, pbuf, 2);
    mov16_ext((Word16)(ntap_qmf_half-1), ps_dif, 1, pbuf+1, 2);
  }
  ELSE {                            /*ovflag==0 && ovflag_pre!=0*/
    array_oper_ext((Word16)(ntap_qmf_half-1), 1, ps_sum, 1, pbuf, 2, &shl);
    array_oper_ext((Word16)(ntap_qmf_half-1), 1, ps_dif, 1, pbuf+1, 2, &shl);
  }

  work->ovflag_pre = ovflag;
  move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/
  return;
}
