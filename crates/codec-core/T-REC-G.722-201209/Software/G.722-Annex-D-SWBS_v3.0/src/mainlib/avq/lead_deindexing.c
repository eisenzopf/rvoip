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

#include "dsputil.h"
#include "rom.h"

/*****************************/
#ifdef DYN_RAM_CNT
#include "dyn_ram_cnt.h"
#endif
/*****************************/

/*-------------------------------------------------------------------*
* Local function prototype
*-------------------------------------------------------------------*/
static void fcb_decode_pos(Word16 index, Word16 pos_vector[], Word16 pulse_num, Word16 pos_num);

static Word16 search_ka(UWord16 I, const UWord16 *ptr0, const Word16 *ptr1, Word16 nb) {
  Word16 i,j;

  Word32 tmp;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) (2 * SIZE_Word16 + 1 * SIZE_Word32), "dummy"); 
#endif 
  /*****************************/

  i = 0; move16();
  FOR(j=0; j<nb; j++)
  {
    tmp = L_sub(I,*(++ptr0));
    if(tmp >= 0 )
    {
      i = add(i, 1);
    }
    IF(tmp < 0)
        BREAK;
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
  move16();
  return(ptr1[i]);
}

/*-------------------------------------------------------------------*
* re8_decode_base_index
*
* Decode RE8 base index
*-------------------------------------------------------------------*/
void re8_decode_base_index(
                           Word16 n,  /* i  : codebook number (*n is an integer defined in {0,2,3,4,..,n_max}) */
                           UWord16 I,  /* i  : index of c (pointer to unsigned 16-bit word)                     */
                           Word16 *x  /* o  : point in RE8 (8-dimensional integer vector)                      */
                           )
{
  Word16 i,j,k1,m,m1;
  Word16 setor_8p_temp[8],setor_8p_temp_1[8];
  Word16 setor_8p_temp_2 = 0; 
  Word16 sign_8p;
  Word16 code_level;
  const Word16 *a1,*a2;
  Word16 ka;
  Word16 code_index;
  Word16 *ptr;
  move16();
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((26 * SIZE_Word16) + (3 * SIZE_Ptr)), "dummy");
#endif

  /*****************************/
  IF (sub( n, 2 ) < 0)
  {
    zero16_8(x);
  }
  ELSE
  {
    if ( L_sub( I, 65519 ) > 0 )
    {
      I = 0; move16();
    }

    /*-------------------------------------------------------------------*
    * search for the identifier ka of the absolute leader (table-lookup)
    * Q2 is a subset of Q3 - the two cases are considered in the same branch
    *-------------------------------------------------------------------*/
    IF (sub(n,3) <= 0 )
    {
      ka = search_ka(I, I3_, A3_, NB_LDQ3);
    }
    ELSE
    {
      ka = search_ka(I, I4_, A4_, NB_LDQ4);
    }
    /*-------------------------------------------------------*
    * decode
    *-------------------------------------------------------*/
    a1 = Vals_a[ka];        move16();
    a2 = Vals_q[ka];        move16();
    k1 = a2[0];             move16();
    code_level = a2[1];     move16();

    code_index = extract_l( L_sub( I, IS_new[ka] ) );

    sign_8p = s_and(code_index, sub(shl(1,k1),1));

    code_index = shr(code_index, k1);

    FOR(i=0; i<8; i++)
    {
      x[i] = a1[0];         move16();
    }

    IF(sub(code_level,1) > 0)
    {
      m = a2[2];                  move16();
      m1 = a2[3];                 move16();
      SWITCH (code_level)
      {
        case 4:
          setor_8p_temp_2 = s_and(code_index, 1);
          code_index = shr(code_index, 1);

        case 3:
          j = extract_l_L_shr(L_mult0(code_index, DIV_mult[Select_table22[m1][m]]), DIV_shift[Select_table22[m1][m]]);
          code_index = sub(code_index, extract_l(L_mult0(j, Select_table22[m1][m])));

          fcb_decode_pos(code_index,setor_8p_temp_1, m, m1);
          code_index = j;             move16();
        case 2:
          fcb_decode_pos(code_index,setor_8p_temp,8,m);
      }

      FOR (i=0;i<m;i++)
      {
        x[setor_8p_temp[i]] = a1[1];  move16(); 
      }

      IF(sub(code_level, 2) > 0)
      {
        FOR (i=0;i<m1;i++)
        {
          x[setor_8p_temp[setor_8p_temp_1[i]]] = a1[2];     move16(); 
        }
        IF(sub(code_level, 3) > 0)
        {
          x[setor_8p_temp[setor_8p_temp_1[setor_8p_temp_2]]] = 6; move16();
        }
      }
    }

    /*--------------------------------------------------------------------*
    * add the sign of all element ( except the last one in some case )
    *--------------------------------------------------------------------*/
    ptr = x+ 7;
    IF(s_and(x[0], 1) != 0)
    {
      /* odd covectors */
      m1 = *ptr--; move16();
      /*--------------------------------------------------------------------*
      * add the sign of all elements  except the last one 
      *--------------------------------------------------------------------*/
      FOR (i=0; i<7; i++)
      {
        if (s_and(sign_8p, 1) != 0)
        {
          *ptr = negate (*ptr);    move16();
        }
        sign_8p = shr(sign_8p, 1); 
        m1 = add(m1, *ptr--);
      }
      /*--------------------------------------------------------------------*
      * recover the sign of last element if needed
      *--------------------------------------------------------------------*/
      if(s_and(m1,3) )
      {
        x[7] = negate(x[7]);
      }
    }
    ELSE 
    {
      /* even covectors */
      /*--------------------------------------------------------------------*
      * add the sign of all non zero elements  
      *--------------------------------------------------------------------*/
      FOR (i=0; i<8; i++)
      {
        IF (*ptr != 0)
        {
          if (s_and(sign_8p, 1) != 0)
          {
            *ptr = negate (*ptr);    move16();
          }
          sign_8p = shr(sign_8p, 1); 
        }
        ptr--;
      }
    }
  }

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

/*-------------------------------------------------------------------*
* fcb_decode_pos
*
* base function for decoding position index
*-------------------------------------------------------------------*/
static void fcb_decode_pos(
                           Word16 index,             /* i  : Index to decoder    */
                           Word16 pos_vector[],      /* o  : Position vector     */
                           Word16 pulse_num,         /* i  : Number of pulses    */
                           Word16 pos_num            /* i  : Number of positions */
                           )
{
  Word16 i,k,l;
  Word16 temp1,temp2;
  const Word16 *select_table23;
  const Word16 *select_table24;
  Word16 temp;
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH((UWord32) ((6 * SIZE_Word16) + (2 * SIZE_Word32)), "dummy");
#endif
  /*****************************/
  k = index;                      move16();
  l = 0;                          move16();
  temp1 = pos_num;                move16();
  temp2 = add( pulse_num, 1 );
  temp = sub(pos_num,1);

  i = 0; move16();
  temp = sub(temp1, temp);
  FOR (temp1 = temp1; temp1 > temp; temp1--)
  {
    select_table23 = Select_table22[temp1] ;                move16();
    select_table24 = &select_table23[sub(pulse_num,l)];     move16();

    k = sub(*select_table24,k);                             move16();
    WHILE (sub( k, *select_table24 ) <= 0 )
    {
      l = add(l,1);
      select_table24--;
    }

    k = sub(select_table23[sub(temp2,l)], k);
    pos_vector[i++] = sub(l,1);                               move16();
  }

  pos_vector[i] = add(l,k);                                   move16();

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 

}

