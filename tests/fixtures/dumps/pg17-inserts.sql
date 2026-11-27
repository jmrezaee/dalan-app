--
-- PostgreSQL database dump
--

\restrict iuTTgH7JWGsBfPtIabDOO4zNCBihp3ZT5CfZf7bPo3KH9ggx20LVzJr4V0lYmcX

-- Dumped from database version 17.12
-- Dumped by pg_dump version 17.12

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: mood; Type: TYPE; Schema: public; Owner: postgres
--

CREATE TYPE public.mood AS ENUM (
    'happy',
    'sad'
);


ALTER TYPE public.mood OWNER TO postgres;

--
-- Name: order_count(integer); Type: FUNCTION; Schema: public; Owner: postgres
--

CREATE FUNCTION public.order_count(c integer) RETURNS bigint
    LANGUAGE sql
    AS $$ select count(*) from orders where customer_id = c $$;


ALTER FUNCTION public.order_count(c integer) OWNER TO postgres;

--
-- Name: touch(); Type: FUNCTION; Schema: public; Owner: postgres
--

CREATE FUNCTION public.touch() RETURNS trigger
    LANGUAGE plpgsql
    AS $$ begin new.note := coalesce(new.note, 'n/a'); return new; end $$;


ALTER FUNCTION public.touch() OWNER TO postgres;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: orders; Type: TABLE; Schema: public; Owner: postgres
--

CREATE TABLE public.orders (
    id bigint NOT NULL,
    customer_id integer,
    total numeric(10,2),
    note text
);


ALTER TABLE public.orders OWNER TO postgres;

--
-- Name: big_orders; Type: VIEW; Schema: public; Owner: postgres
--

CREATE VIEW public.big_orders AS
 SELECT id,
    customer_id,
    total,
    note
   FROM public.orders
  WHERE (total > (100)::numeric);


ALTER VIEW public.big_orders OWNER TO postgres;

--
-- Name: customers; Type: TABLE; Schema: public; Owner: app_owner
--

CREATE TABLE public.customers (
    id integer NOT NULL,
    name text NOT NULL,
    email text,
    mood public.mood,
    created timestamp with time zone DEFAULT now()
);


ALTER TABLE public.customers OWNER TO app_owner;

--
-- Name: customers_id_seq; Type: SEQUENCE; Schema: public; Owner: app_owner
--

CREATE SEQUENCE public.customers_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


ALTER SEQUENCE public.customers_id_seq OWNER TO app_owner;

--
-- Name: customers_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: app_owner
--

ALTER SEQUENCE public.customers_id_seq OWNED BY public.customers.id;


--
-- Name: orders_id_seq; Type: SEQUENCE; Schema: public; Owner: postgres
--

ALTER TABLE public.orders ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.orders_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Name: customers id; Type: DEFAULT; Schema: public; Owner: app_owner
--

ALTER TABLE ONLY public.customers ALTER COLUMN id SET DEFAULT nextval('public.customers_id_seq'::regclass);


--
-- Data for Name: customers; Type: TABLE DATA; Schema: public; Owner: app_owner
--

INSERT INTO public.customers VALUES (1, 'Ada', 'ada@example.test', 'happy', '2026-11-27 14:30:12.789585+00');
INSERT INTO public.customers VALUES (2, 'Bo', 'bo@example.test', 'sad', '2026-11-27 14:30:12.789585+00');
INSERT INTO public.customers VALUES (3, 'Cy', NULL, NULL, '2026-11-27 14:30:12.789585+00');


--
-- Data for Name: orders; Type: TABLE DATA; Schema: public; Owner: postgres
--

INSERT INTO public.orders OVERRIDING SYSTEM VALUE VALUES (1, 1, 120.50, 'tab\there');
INSERT INTO public.orders OVERRIDING SYSTEM VALUE VALUES (2, 1, 9.99, 'line
break');
INSERT INTO public.orders OVERRIDING SYSTEM VALUE VALUES (3, 2, 300.00, 'back\\slash');
INSERT INTO public.orders OVERRIDING SYSTEM VALUE VALUES (4, 3, 1.00, 'n/a');


--
-- Name: customers_id_seq; Type: SEQUENCE SET; Schema: public; Owner: app_owner
--

SELECT pg_catalog.setval('public.customers_id_seq', 3, true);


--
-- Name: orders_id_seq; Type: SEQUENCE SET; Schema: public; Owner: postgres
--

SELECT pg_catalog.setval('public.orders_id_seq', 4, true);


--
-- Name: customers customers_email_key; Type: CONSTRAINT; Schema: public; Owner: app_owner
--

ALTER TABLE ONLY public.customers
    ADD CONSTRAINT customers_email_key UNIQUE (email);


--
-- Name: customers customers_pkey; Type: CONSTRAINT; Schema: public; Owner: app_owner
--

ALTER TABLE ONLY public.customers
    ADD CONSTRAINT customers_pkey PRIMARY KEY (id);


--
-- Name: orders orders_pkey; Type: CONSTRAINT; Schema: public; Owner: postgres
--

ALTER TABLE ONLY public.orders
    ADD CONSTRAINT orders_pkey PRIMARY KEY (id);


--
-- Name: orders orders_touch; Type: TRIGGER; Schema: public; Owner: postgres
--

CREATE TRIGGER orders_touch BEFORE INSERT ON public.orders FOR EACH ROW EXECUTE FUNCTION public.touch();


--
-- Name: orders orders_customer_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: postgres
--

ALTER TABLE ONLY public.orders
    ADD CONSTRAINT orders_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES public.customers(id);


--
-- Name: TABLE orders; Type: ACL; Schema: public; Owner: postgres
--

GRANT SELECT ON TABLE public.orders TO app_owner;


--
-- PostgreSQL database dump complete
--

\unrestrict iuTTgH7JWGsBfPtIabDOO4zNCBihp3ZT5CfZf7bPo3KH9ggx20LVzJr4V0lYmcX

